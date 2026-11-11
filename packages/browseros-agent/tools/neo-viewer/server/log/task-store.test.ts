import { afterEach, expect, test } from 'bun:test'
import { appendFile, mkdtemp, readdir, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { TaskRule } from '../tasks/task-rule'
import type { StoredRunEnd } from './task-log'
import { type RunStart, TaskStore } from './task-store'

const dirs: string[] = []
const stores: TaskStore[] = []
afterEach(async () => {
  for (const store of stores.splice(0)) await store.flush()
  for (const dir of dirs.splice(0))
    await rm(dir, { recursive: true, force: true })
})

async function open(dataDir: string) {
  const rule = new TaskRule(() => 'task-1')
  const store = await TaskStore.open(dataDir, rule)
  stores.push(store)
  return { rule, store }
}

function start(store: TaskStore, sentAt: number, code: string): RunStart {
  return {
    arrival: store.nextArrival(),
    tool: 'run',
    code,
    timeoutMs: null,
    taskIdStamped: true,
    sentAt: new Date(sentAt).toISOString(),
    proxyMs: 0,
    request: { raw: '{}', forwarded: null, jsonRpcId: 1 },
  }
}

function end(answeredAt: number): StoredRunEnd {
  return {
    status: 'ok',
    answeredAt: new Date(answeredAt).toISOString(),
    reply: {
      httpStatus: 200,
      contentType: 'text/event-stream',
      contentEncoding: null,
      raw: '',
      text: '1',
      structuredContent: null,
      isError: false,
      meta: null,
      rpcError: null,
      notifications: [],
      keepAlives: 0,
    },
  }
}

async function withTask(identity: 'process' | 'agent' = 'process') {
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-store-'))
  dirs.push(dataDir)
  const { rule, store } = await open(dataDir)
  const assignment = rule.assign({
    agentTaskId: identity === 'agent' ? 'zuzu-7' : null,
    client: { name: 'claude-code' },
    process: { found: false, reason: 'test' },
    at: 0,
  })
  store.createTask(assignment, { name: 'claude-code' }, 0)
  return { dataDir, store, taskId: assignment.taskId }
}

test('runs are listed in the order they arrived, whatever order they were recorded in', async () => {
  const { store, taskId } = await withTask()
  // The second call's task was known first (a slow process lookup for the first).
  const firstStart = start(store, 1_000, 'return 1')
  const secondStart = start(store, 3_000, 'return 2')
  const second = store.startRun(taskId, secondStart)
  const first = store.startRun(taskId, firstStart)
  store.endRun(first, end(2_000))
  store.endRun(second, end(3_500))
  const runs = store.task(taskId)?.runs ?? []
  expect(runs.map((run) => [run.seq, run.code, run.gapMs])).toEqual([
    [1, 'return 1', null],
    [2, 'return 2', 1_000],
  ])
})

test('a run sent while an earlier one was going has no gap', async () => {
  const { store, taskId } = await withTask()
  const first = store.startRun(taskId, start(store, 1_000, 'a'))
  const second = store.startRun(taskId, start(store, 1_500, 'b'))
  expect(store.task(taskId)?.runs[1]?.gapMs).toBeNull()
  store.endRun(first, end(2_000))
  store.endRun(second, end(2_500))
  expect(store.task(taskId)?.runs[1]?.gapMs).toBeNull()
})

test('a torn last line from a crash costs only itself, across restarts', async () => {
  const { dataDir, store, taskId } = await withTask()
  const run = store.startRun(taskId, start(store, 1_000, 'before the crash'))
  store.endRun(run, end(1_100))
  await store.flush()
  const [file] = await readdir(join(dataDir, 'tasks'))
  await appendFile(
    join(dataDir, 'tasks', file ?? ''),
    '{"type":"run-start","run":{"id":"to',
  )

  const second = await open(dataDir)
  const after = second.store.startRun(
    taskId,
    start(second.store, 2_000, 'after the crash'),
  )
  second.store.endRun(after, end(2_100))
  await second.store.flush()

  const third = await open(dataDir)
  const runs = third.store.task(taskId)?.runs ?? []
  expect(runs.map((listed) => [listed.code, listed.status])).toEqual([
    ['before the crash', 'ok'],
    ['after the crash', 'ok'],
  ])
})

test('a reply recorded late gives the later run its gap, and says so', async () => {
  const { store, taskId } = await withTask()
  const events: Array<[number, string, number | null]> = []
  store.subscribe((event) => {
    if (event.type === 'run')
      events.push([event.version, event.run.code, event.run.gapMs])
  })
  const firstStart = start(store, 1_000, 'a')
  const second = store.startRun(taskId, start(store, 3_000, 'b'))
  const first = store.startRun(taskId, firstStart)
  store.endRun(second, end(3_500))
  store.endRun(first, end(2_000))
  const last = events.filter(([, code]) => code === 'b').at(-1)
  expect(last?.[2]).toBe(1_000)
  // Every event is numbered, in order, and a snapshot reports the latest number.
  expect(events.map(([version]) => version)).toEqual(
    [...events.map(([version]) => version)].sort((x, y) => x - y),
  )
  expect(store.version).toBeGreaterThanOrEqual(events.at(-1)?.[0] ?? 0)
})

test("a tools/list logged after the agent's first run still reaches that run's task", async () => {
  const { store, taskId } = await withTask('agent')
  store.startRun(taskId, start(store, 1_000, 'a'))
  // The agent behind the run: rule 3's key for client claude-code (the process was not found).
  store.toolsListSeen(
    'client:claude-code',
    { chars: 6_562, tokens: 1_640 },
    900,
  )
  expect(store.task(taskId)?.task.toolsList).toEqual({
    chars: 6_562,
    tokens: 1_640,
  })
})
