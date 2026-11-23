/**
 * Precedence, as the viewer guarantees it: which record source the joiner gets is chosen once (a
 * host API, or spans), so whatever order a run's reply, Neo's host record and its spans arrive in,
 * `run.neo` goes from nothing to one source and stays there, and its operations never shrink or
 * repeat. These drive the real store, joiner and telemetry; only Neo is played.
 */

import { afterEach, describe, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { RunTraceContext } from '../../shared/api'
import type { StoredRunEnd } from '../log/run-log'
import { RunStore } from '../log/run-store'
import { hostApiRecords, joinNeoRuns, toNeoRunRecord } from '../neo-runs'
import { startTelemetry } from '.'
import { fixtureHostRun, fixtureTraces } from './fixtures'
import { decodeSpans } from './otlp-json'
import { spanRecords } from './span-records'
import { SpanStore } from './span-store'

type Mode = 'host-api' | 'spans-only'
type Arrival = 'reply' | 'host-record' | 'op-spans' | 'run-span'

const MINTED: RunTraceContext = {
  traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
  spanId: '00f067aa0ba902b7',
  injected: true,
}
const RUN = {
  neoRunId: 'r-fixture',
  traceId: MINTED.traceId,
  parentSpanId: MINTED.spanId,
  startMs: Date.parse('2026-11-20T12:00:00Z'),
}

const dirs: string[] = []
afterEach(async () => {
  for (const dir of dirs.splice(0))
    await rm(dir, { recursive: true, force: true })
})

function ended(neoRunId: string | null): StoredRunEnd {
  return {
    status: neoRunId ? 'ok' : 'cancelled',
    answeredAt: new Date().toISOString(),
    reply: {
      httpStatus: neoRunId ? 200 : null,
      contentType: null,
      contentEncoding: null,
      raw: null,
      text: neoRunId ? '1' : null,
      structuredContent: null,
      isError: neoRunId ? false : null,
      meta: neoRunId ? { 'com.browseros.neo/run': { id: neoRunId } } : null,
      rpcError: null,
      notifications: [],
      keepAlives: 0,
    },
  }
}

/** A viewer whose one run meets these arrivals in this order; each span batch is sent twice. */
async function play(mode: Mode, arrivals: Arrival[]) {
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-records-'))
  dirs.push(dataDir)
  const store = await RunStore.open(dataDir)
  const runId = store.startRun({
    arrival: store.nextArrival(),
    tool: 'run',
    code: 'return 1',
    timeoutMs: null,
    client: null,
    agentTaskId: null,
    trace: MINTED,
    sentAt: new Date().toISOString(),
    proxyMs: 0,
    request: { raw: '{}', jsonRpcId: 1 },
  })
  const telemetry = startTelemetry({
    hostApi: mode === 'host-api',
    now: Date.now,
    scrape: { fetch: async () => new Response(null, { status: 404 }) },
    readWaitMs: 300,
    firstSpanGraceMs: 120,
  })
  let hostRecord = false
  // Neo's `GET /v1/runs/{id}?wait=…`: answers once it has the record, or gives up.
  const host = hostApiRecords(async () => {
    const until = Date.now() + 300
    while (!hostRecord && Date.now() < until) await Bun.sleep(5)
    return hostRecord ? toNeoRunRecord(fixtureHostRun(RUN)) : null
  })
  const joiner = joinNeoRuns(
    store,
    mode === 'host-api' ? host : telemetry.records,
  )
  const seen: Array<{ source: string | null; ops: number }> = []
  store.subscribe((event) => {
    if (event.type !== 'run') return
    const neo = store.run(runId)?.neo ?? null
    seen.push({
      source: neo ? (neo.source ?? 'host') : null,
      ops: neo?.operations.length ?? 0,
    })
  })
  const route = telemetry.receiverRoutes['/otlp/v1/traces']
  const push = async (ops: number[]) => {
    for (let copy = 0; copy < 2; copy++) {
      const response = await route?.POST(
        new Request('http://127.0.0.1:7391/otlp/v1/traces', {
          method: 'POST',
          headers: {
            'content-type': 'application/json',
            host: '127.0.0.1:7391',
          },
          body: JSON.stringify(fixtureTraces(RUN, { ops })),
        }),
      )
      expect(response?.status).toBe(200)
    }
  }
  // No reply: the agent went away and the viewer ended the run as cancelled.
  if (!arrivals.includes('reply')) store.endRun(runId, ended(null))
  for (const arrival of arrivals) {
    if (arrival === 'reply') store.endRun(runId, ended(RUN.neoRunId))
    else if (arrival === 'host-record') hostRecord = true
    else if (arrival === 'op-spans') await push([1, 2, 3, 4, 5, 6, 7, 8])
    else await push([0])
    await Bun.sleep(15)
  }
  await joiner.settled(1_500)
  const neo = store.run(runId)?.neo ?? null
  await telemetry.stop()
  await joiner.stop()
  await store.flush()
  return { seen, neo, store, runId }
}

/** `null* X*` for one X, and operations never fewer than before. */
function monotone(seen: Array<{ source: string | null; ops: number }>) {
  const sources = seen.map((entry) => entry.source).filter((s) => s !== null)
  expect(new Set(sources).size).toBeLessThanOrEqual(1)
  const firstSet = seen.findIndex((entry) => entry.source !== null)
  if (firstSet >= 0)
    expect(seen.slice(firstSet).every((entry) => entry.source !== null)).toBe(
      true,
    )
  for (let i = 1; i < seen.length; i++)
    expect(seen[i]?.ops ?? 0).toBeGreaterThanOrEqual(seen[i - 1]?.ops ?? 0)
}

const PRECEDENCE_TABLE: ReadonlyArray<{
  mode: Mode
  arrived: Arrival[]
  expect: 'host' | 'spans' | null
}> = [
  {
    mode: 'host-api',
    arrived: ['reply', 'host-record', 'op-spans', 'run-span'],
    expect: 'host',
  },
  // With a host API, spans never fill the op table.
  {
    mode: 'host-api',
    arrived: ['reply', 'op-spans', 'run-span'],
    expect: null,
  },
  { mode: 'host-api', arrived: ['op-spans'], expect: null },
  // Spans before the reply, and after it.
  {
    mode: 'spans-only',
    arrived: ['op-spans', 'run-span', 'reply'],
    expect: 'spans',
  },
  {
    mode: 'spans-only',
    arrived: ['reply', 'op-spans', 'run-span'],
    expect: 'spans',
  },
  // An older image: exactly today ("Calls –").
  { mode: 'spans-only', arrived: ['reply'], expect: null },
  // A cancelled run (no reply) found by the span id the viewer minted for it.
  { mode: 'spans-only', arrived: ['run-span'], expect: 'spans' },
]

describe('the precedence table', () => {
  test.each(
    PRECEDENCE_TABLE.map((row) => [row.mode, row.arrived.join(' → '), row]),
  )('%s, %s', async (_, __, row) => {
    const { seen, neo } = await play(row.mode, row.arrived)
    monotone(seen)
    expect(neo ? (neo.source ?? 'host') : null).toBe(row.expect)
  })
})

function permutations<T>(items: T[]): T[][] {
  if (items.length <= 1) return [items]
  return items.flatMap((item, index) =>
    permutations([...items.slice(0, index), ...items.slice(index + 1)]).map(
      (rest) => [item, ...rest],
    ),
  )
}

describe('every arrival order, every span batch delivered twice: one source, operations only grow', () => {
  for (const mode of ['host-api', 'spans-only'] as const) {
    test(mode, async () => {
      const orders = permutations<Arrival>([
        'reply',
        'host-record',
        'op-spans',
        'run-span',
      ])
      expect(orders).toHaveLength(24)
      const results = await Promise.all(
        orders.map((order) => play(mode, order)),
      )
      for (const { seen, neo } of results) {
        monotone(seen)
        expect(neo?.source ?? 'host').toBe(
          mode === 'host-api' ? 'host' : 'spans',
        )
        expect(neo?.complete).toBe(true)
        expect(neo?.operations).toHaveLength(8)
        // No operation twice.
        expect(new Set(neo?.operations.map((op) => op.id)).size).toBe(8)
      }
    }, 30_000)
  }
})

test('a span-built record is kept in the log: a restart shows the same operations', async () => {
  const { store, runId, neo } = await play('spans-only', [
    'reply',
    'op-spans',
    'run-span',
  ])
  expect(neo?.source).toBe('spans')
  const dataDir = dirs.at(-1) ?? ''
  const reopened = await RunStore.open(dataDir)
  expect(reopened.run(runId)?.neo).toEqual(store.run(runId)?.neo ?? null)
  expect(reopened.list()[0]?.operationCount).toBe(8)
})

test("two runs under one caller trace id keep their own spans: the key is Neo's run id, never the trace", async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-records-'))
  dirs.push(dataDir)
  const store = await RunStore.open(dataDir)
  const caller: RunTraceContext = { ...MINTED, injected: false }
  const telemetry = startTelemetry({
    hostApi: false,
    now: Date.now,
    readWaitMs: 300,
  })
  const joiner = joinNeoRuns(store, telemetry.records)
  const runs = ['run-a', 'run-b'].map((neoRunId) => {
    const runId = store.startRun({
      arrival: store.nextArrival(),
      tool: 'run',
      code: 'return 1',
      timeoutMs: null,
      client: null,
      agentTaskId: null,
      trace: caller,
      sentAt: new Date().toISOString(),
      proxyMs: 0,
      request: { raw: '{}', jsonRpcId: 1 },
    })
    return { runId, neoRunId }
  })
  const post = (body: unknown) =>
    telemetry.receiverRoutes['/otlp/v1/traces']?.POST(
      new Request('http://localhost/otlp/v1/traces', {
        method: 'POST',
        headers: { 'content-type': 'application/json', host: '127.0.0.1:7391' },
        body: JSON.stringify(body),
      }),
    )
  // Same trace id, same parent: only `neo.run.id` tells them apart.
  await post(fixtureTraces({ ...RUN, neoRunId: 'run-a' }, { ops: [0, 1, 2] }))
  await post(fixtureTraces({ ...RUN, neoRunId: 'run-b' }, { ops: [0, 3] }))
  for (const { runId, neoRunId } of runs) store.endRun(runId, ended(neoRunId))
  await joiner.settled(1_000)
  const [a, b] = runs.map(({ runId }) => store.run(runId))
  expect(a?.neo?.operations.map((op) => op.id)).toEqual(['run-a.1', 'run-a.2'])
  expect(b?.neo?.operations.map((op) => op.id)).toEqual(['run-b.3'])
  const traceOf = (neoRunId: string) =>
    telemetry
      .runTrace({ neoRunId, mintedSpanId: null })
      ?.spans.map((span) => span.label)
  expect(traceOf('run-a')).toEqual([
    'run-a',
    'page.goto',
    'locator.textContent',
  ])
  expect(traceOf('run-b')).toEqual(['run-b', 'locator.count'])
  await telemetry.stop()
  await joiner.stop()
  await store.flush()
})

test('a neo.run that comes after the reads ran out is read once more', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-records-'))
  dirs.push(dataDir)
  const store = await RunStore.open(dataDir)
  const telemetry = startTelemetry({
    hostApi: false,
    now: Date.now,
    readWaitMs: 40,
  })
  const joiner = joinNeoRuns(store, telemetry.records)
  const runId = store.startRun({
    arrival: store.nextArrival(),
    tool: 'run',
    code: 'return 1',
    timeoutMs: null,
    client: null,
    agentTaskId: null,
    trace: MINTED,
    sentAt: new Date().toISOString(),
    proxyMs: 0,
    request: { raw: '{}', jsonRpcId: 1 },
  })
  const post = (ops: number[]) =>
    telemetry.receiverRoutes['/otlp/v1/traces']?.POST(
      new Request('http://localhost/otlp/v1/traces', {
        method: 'POST',
        headers: { 'content-type': 'application/json', host: '127.0.0.1:7391' },
        body: JSON.stringify(fixtureTraces(RUN, { ops })),
      }),
    )
  await post([1, 2])
  store.endRun(runId, ended(RUN.neoRunId))
  // Three reads of 40 ms each find no neo.run: the record stays incomplete.
  await Bun.sleep(250)
  expect(store.run(runId)?.neo).toMatchObject({ complete: false })
  await post([0, 3, 4, 5, 6, 7, 8])
  await joiner.settled(1_000)
  expect(store.run(runId)?.neo).toMatchObject({ complete: true, status: 'ok' })
  expect(store.run(runId)?.operationCount).toBe(8)
  await telemetry.stop()
  await joiner.stop()
  await store.flush()
})

test('a late span for a run the store already dropped never announces a smaller record', async () => {
  const spans = new SpanStore({
    maxRuns: 2,
    maxSpansPerRun: 100,
    maxSpans: 1e6,
    maxBytes: 1e9,
  })
  const records = spanRecords(spans, { now: Date.now, waitMs: 30 })
  const announced: Array<string | null> = []
  records.onLateCompletion?.((key) => announced.push(key.neoRunId))
  const add = (neoRunId: string, ops: number[]) =>
    records.changed(
      spans.add(decodeSpans(fixtureTraces({ ...RUN, neoRunId }, { ops })).items)
        .changed,
    )
  const read = (neoRunId: string) =>
    records.read({ neoRunId, trace: null }, new AbortController().signal)
  add('r1', [0, 1, 2, 3, 4, 5, 6, 7, 8])
  expect((await read('r1'))?.operations).toHaveLength(8)
  // Two newer runs push r1 out; then an exporter's retry of one of its ops arrives.
  add('r2', [0])
  add('r3', [0])
  expect(spans.get('r1')).toBeNull()
  add('r1', [3])
  expect(announced).toEqual([])
  // A record that grows is still announced: r4 read before its neo.run came.
  add('r4', [1, 2])
  expect((await read('r4'))?.complete).toBe(false)
  add('r4', [0, 3])
  expect(announced).toEqual(['r4'])
})

test('a run whose first spans come after its read gave up empty is read again when they arrive', async () => {
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-records-'))
  dirs.push(dataDir)
  const store = await RunStore.open(dataDir)
  // The very first spans of a viewer's life, slower than the first-span grace.
  const telemetry = startTelemetry({
    hostApi: false,
    now: Date.now,
    firstSpanGraceMs: 20,
  })
  const joiner = joinNeoRuns(store, telemetry.records)
  const runId = store.startRun({
    arrival: store.nextArrival(),
    tool: 'run',
    code: 'return 1',
    timeoutMs: null,
    client: null,
    agentTaskId: null,
    trace: MINTED,
    sentAt: new Date().toISOString(),
    proxyMs: 0,
    request: { raw: '{}', jsonRpcId: 1 },
  })
  store.endRun(runId, ended(RUN.neoRunId))
  await joiner.settled(1_000)
  expect(store.run(runId)?.neo).toBeNull()
  await telemetry.receiverRoutes['/otlp/v1/traces']?.POST(
    new Request('http://127.0.0.1/otlp/v1/traces', {
      method: 'POST',
      headers: { 'content-type': 'application/json', host: '127.0.0.1' },
      body: JSON.stringify(fixtureTraces(RUN)),
    }),
  )
  await joiner.settled(1_000)
  expect(store.run(runId)?.neo).toMatchObject({
    source: 'spans',
    complete: true,
  })
  expect(store.run(runId)?.operationCount).toBe(8)
  await telemetry.stop()
  await joiner.stop()
  await store.flush()
})
