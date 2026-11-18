import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { NeoRunRecord, RunSummary } from '../shared/api'
import { RunStore } from './log/run-store'
import { joinNeoRuns, toNeoRunRecord } from './neo-runs'

/** A `GET /v1/runs/{id}` body as Neo returns it (openapi.yaml `Run`), with one nested call. */
const RUN = {
  id: '01a1',
  taskId: 'claude-code-1',
  status: 'error',
  effects: 'done',
  startedAt: 1_000,
  endedAt: 1_900,
  operationCount: 3,
  complete: true,
  tabs: [7],
  error: {
    owner: 'script',
    code: 'no_match',
    message: 'locator.click: Timeout 500ms exceeded.',
    retryable: true,
    operationId: '01a1.3',
  },
  operations: [
    {
      id: '01a1.1',
      runId: '01a1',
      name: 'page.goto',
      kind: 'builtin',
      effect: 'write',
      site: { line: 1, column: 12 },
      tabId: 7,
      args: { url: 'https://example.com' },
      argsTruncated: false,
      result: null,
      outcome: 'ok',
      delivery: 'done',
      notes: ['navigating to "https://example.com"'],
      startedAt: 1_010,
      endedAt: 1_300,
      durationMs: 290,
      hasScreenshot: true,
    },
    {
      id: '01a1.2',
      runId: '01a1',
      parentId: '01a1.1',
      name: 'frame.waitForLoadState',
      kind: 'builtin',
      effect: 'read',
      startedAt: 1_050,
      durationMs: 200,
      hasScreenshot: false,
    },
    {
      id: '01a1.3',
      runId: '01a1',
      name: 'locator.click',
      kind: 'builtin',
      effect: 'write',
      site: { line: 2, column: 50 },
      startedAt: 1_350,
      endedAt: 1_890,
      durationMs: 540,
      outcome: 'error',
      delivery: 'not_sent',
      hasScreenshot: true,
      error: {
        owner: 'script',
        code: 'no_match',
        message: 'locator.click: Timeout 500ms exceeded.',
        retryable: true,
      },
    },
  ],
}

test("Neo's run record keeps each operation's name, site, timing, args, result, notes, delivery and error", () => {
  const record = toNeoRunRecord(RUN)
  expect(record).toMatchObject({
    status: 'error',
    effects: 'done',
    durationMs: 900,
    complete: true,
    tabs: [7],
    traceGaps: 0,
  })
  // Neo sends traceGaps only when it dropped records.
  expect(toNeoRunRecord({ ...RUN, traceGaps: 2 }).traceGaps).toBe(2)
  expect(record.error).toMatchObject({
    owner: 'script',
    code: 'no_match',
    operationId: '01a1.3',
  })
  expect(
    record.operations.map((op) => [
      op.seq,
      op.name,
      op.site,
      op.startOffsetMs,
      op.durationMs,
      op.effect,
    ]),
  ).toEqual([
    [1, 'page.goto', { line: 1, column: 12 }, 10, 290, 'write'],
    [2, 'frame.waitForLoadState', null, 50, 200, 'read'],
    [3, 'locator.click', { line: 2, column: 50 }, 350, 540, 'write'],
  ])
  expect(record.operations[0]).toMatchObject({
    args: { url: 'https://example.com' },
    outcome: 'ok',
    delivery: 'done',
    notes: ['navigating to "https://example.com"'],
    hasScreenshot: true,
  })
  expect(record.operations[1]).toMatchObject({
    parentId: '01a1.1',
    args: null,
    outcome: null,
    delivery: null,
    notes: [],
  })
  expect(record.operations[2]?.error).toMatchObject({
    code: 'no_match',
    retryable: true,
  })
})

test('values markers distinguish omitted values and preserve future recording policies', () => {
  const record = toNeoRunRecord({
    ...RUN,
    values: 'off',
    operations: [
      {
        id: '01a1.1',
        name: 'locator.fill',
        values: 'off',
        kind: 'builtin',
        effect: 'write',
        tabId: 7,
        startedAt: 1_010,
        durationMs: 290,
        outcome: 'error',
        delivery: 'not_sent',
        error: { ...RUN.error, message: '', operationId: '01a1.1' },
      },
    ],
  })
  expect(record.values).toBe('off')
  expect(record.operations[0]).toMatchObject({
    values: 'off',
    name: 'locator.fill',
    tabId: 7,
    startOffsetMs: 10,
    durationMs: 290,
    outcome: 'error',
    delivery: 'not_sent',
    args: null,
    result: null,
    notes: [],
    error: { owner: 'script', code: 'no_match', message: '', retryable: true },
  })
  const future = toNeoRunRecord({
    ...RUN,
    values: 'future-policy',
    operations: [{ ...RUN.operations[0], values: 'future-policy' }],
  })
  expect(future.values).toBe('future-policy')
  expect(future.operations[0]?.values).toBe('future-policy')
  expect(toNeoRunRecord(RUN)).not.toHaveProperty('values')
  expect(toNeoRunRecord(RUN).operations[0]).not.toHaveProperty('values')
})

function fakeStore() {
  const attached: Array<[string, NeoRunRecord]> = []
  let listener: ((event: { type: string; run?: RunSummary }) => void) | null =
    null
  return {
    attached,
    emit: (run: Partial<RunSummary>) =>
      listener?.({ type: 'run', run: run as RunSummary }),
    store: {
      subscribe(next: (event: { type: string; run?: RunSummary }) => void) {
        listener = next
        return () => {
          listener = null
        }
      },
      attachNeoRun: (runId: string, record: NeoRunRecord) => {
        attached.push([runId, record])
      },
      hasNeoRun: (runId: string) => attached.some(([id]) => id === runId),
    },
  }
}

test('the joiner reads the record of each run that ended, re-reading one that was not complete', async () => {
  const fake = fakeStore()
  const reads: string[] = []
  let answers = [{ ...RUN, complete: false }, RUN]
  joinNeoRuns(fake.store, async (neoRunId) => {
    reads.push(neoRunId)
    const next = answers[0]
    answers = answers.slice(1)
    return next ? toNeoRunRecord(next) : null
  })
  fake.emit({ id: 'r1', status: 'running', neoRunId: null })
  fake.emit({ id: 'r1', status: 'error', neoRunId: '01a1' })
  fake.emit({ id: 'r1', status: 'error', neoRunId: '01a1' })
  await Bun.sleep(10)
  expect(reads).toEqual(['01a1', '01a1'])
  expect(fake.attached.map(([id, record]) => [id, record.complete])).toEqual([
    ['r1', false],
    ['r1', true],
  ])
})

test('a run without a Neo run id, or whose record cannot be read, keeps no record', async () => {
  const fake = fakeStore()
  joinNeoRuns(fake.store, async () => null)
  fake.emit({ id: 'r1', status: 'http-error', neoRunId: null })
  fake.emit({ id: 'r2', status: 'ok', neoRunId: '01a2' })
  await Bun.sleep(10)
  expect(fake.attached).toEqual([])
})

const dirs: string[] = []
afterEach(async () => {
  for (const dir of dirs.splice(0))
    await rm(dir, { recursive: true, force: true })
})

test('a joined record is in the run detail and the list, and survives a restart', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-join-'))
  dirs.push(dataDir)
  const store = await RunStore.open(dataDir)
  const runId = store.startRun({
    arrival: store.nextArrival(),
    tool: 'run',
    code: 'await page.goto(url)',
    timeoutMs: null,
    client: { name: 'claude-code' },
    agentTaskId: null,
    sentAt: new Date(1_000).toISOString(),
    proxyMs: 0,
    request: { raw: '{}', jsonRpcId: 1 },
  })
  store.attachNeoRun(
    runId,
    toNeoRunRecord({
      ...RUN,
      values: 'off',
      operations: RUN.operations.map((operation) => ({
        ...operation,
        values: 'off',
        args: undefined,
        result: undefined,
        notes: undefined,
      })),
    }),
  )
  await store.flush()
  const reopened = await RunStore.open(dataDir)
  expect(reopened.list()[0]).toMatchObject({
    neoDurationMs: 900,
    operationCount: 3,
  })
  expect(reopened.run(runId)?.neo?.operations.map((op) => op.name)).toEqual([
    'page.goto',
    'frame.waitForLoadState',
    'locator.click',
  ])
  expect(reopened.run(runId)?.neo?.values).toBe('off')
  expect(
    reopened.run(runId)?.neo?.operations.map((operation) => operation.values),
  ).toEqual(['off', 'off', 'off'])
})

test('stopping waits for a read in flight, so its record is attached before the final flush', async () => {
  const fake = fakeStore()
  const joiner = joinNeoRuns(fake.store, async () => {
    await Bun.sleep(50)
    return toNeoRunRecord(RUN)
  })
  fake.emit({ id: 'r1', status: 'ok', neoRunId: '01a1' })
  await joiner.stop()
  expect(fake.attached.map(([id]) => id)).toEqual(['r1'])
})

test('settling waits for a read in flight and keeps joining (an export right after a run)', async () => {
  const fake = fakeStore()
  const joiner = joinNeoRuns(fake.store, async () => {
    await Bun.sleep(50)
    return toNeoRunRecord(RUN)
  })
  fake.emit({ id: 'r1', status: 'ok', neoRunId: '01a1' })
  await joiner.settled(1_000)
  expect(fake.attached.map(([id]) => id)).toEqual(['r1'])
  // Still joining after it settled.
  fake.emit({ id: 'r2', status: 'ok', neoRunId: '01a2' })
  await joiner.settled(1_000)
  expect(fake.attached.map(([id]) => id)).toEqual(['r1', 'r2'])
  // A read slower than the wait does not hold the caller past it.
  const slow = joinNeoRuns(fakeStore().store, () => new Promise(() => {}))
  const started = Date.now()
  await slow.settled(30)
  expect(Date.now() - started).toBeLessThan(500)
})
