import { afterEach, describe, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { RunStore, tasksFromMeta } from './run-store'

/** A reply `_meta` as Neo sends it (#3452), with `tasks` as given. */
const meta = (tasks: unknown) => ({
  'com.browseros.neo/run': { id: 'r1', status: 'ok', effects: 'done', tasks },
})

describe("the tasks a run touched, from the reply's _meta", () => {
  test('none: no _meta, no tasks key (an older Neo), an empty list', () => {
    expect(tasksFromMeta(null)).toEqual([])
    expect(tasksFromMeta({ 'com.browseros.neo/run': { id: 'r1' } })).toEqual([])
    expect(tasksFromMeta(meta([]))).toEqual([])
  })

  test('one: its id and the name it was created with', () => {
    expect(
      tasksFromMeta(
        meta([{ id: 'shop-4b80', kind: 'created', name: 'shop', tabs: [] }]),
      ),
    ).toEqual([{ id: 'shop-4b80', name: 'shop' }])
  })

  test("several: in Neo's order, once each, a missing name as null, anything malformed skipped", () => {
    expect(
      tasksFromMeta(
        meta([
          {
            id: 'shop-4b80',
            kind: 'fetched',
            name: 'shop',
            tabs: [{ tabId: 1 }],
          },
          { id: 't-91c2', kind: 'created', tabs: [] },
          { id: 'shop-4b80', kind: 'fetched', name: 'shop', tabs: [] },
          { kind: 'created' },
          'not a task',
          { id: '', name: 'empty' },
        ]),
      ),
    ).toEqual([
      { id: 'shop-4b80', name: 'shop' },
      { id: 't-91c2', name: null },
    ])
  })
})

describe('a run in the store', () => {
  const dirs: string[] = []
  afterEach(async () => {
    for (const dir of dirs.splice(0))
      await rm(dir, { recursive: true, force: true })
  })

  async function endedRun(agentTaskId: string | null, replyMeta: unknown) {
    const dir = await mkdtemp(join(tmpdir(), 'neo-viewer-tasks-'))
    dirs.push(dir)
    const store = await RunStore.open(dir)
    const id = store.startRun({
      arrival: store.nextArrival(),
      tool: 'run',
      code: 'return 1',
      timeoutMs: null,
      client: { name: 'claude-code' },
      agentTaskId,
      sentAt: new Date().toISOString(),
      proxyMs: 0,
      request: { raw: '{}', jsonRpcId: 1 },
    })
    store.endRun(id, {
      status: 'ok',
      answeredAt: new Date().toISOString(),
      reply: {
        httpStatus: 200,
        contentType: 'text/event-stream',
        contentEncoding: null,
        raw: '',
        text: '### Result\n1',
        structuredContent: null,
        isError: false,
        meta: replyMeta,
        rpcError: null,
        notifications: [],
        keepAlives: 0,
      },
    })
    await store.flush()
    return { store, id }
  }

  test("a host task stays the request's agentTaskId; the script's tasks are tasks, in the list and in full", async () => {
    const host = await endedRun('host-1', meta([]))
    expect(host.store.list()[0]).toMatchObject({
      agentTaskId: 'host-1',
      tasks: [],
    })
    const scripted = await endedRun(
      null,
      meta([{ id: 'shop-4b80', name: 'shop', tabs: [] }]),
    )
    expect(scripted.store.list()[0]?.tasks).toEqual([
      { id: 'shop-4b80', name: 'shop' },
    ])
    expect(scripted.store.run(scripted.id)?.tasks).toEqual([
      { id: 'shop-4b80', name: 'shop' },
    ])
  })
})
