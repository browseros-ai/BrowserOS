import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  apiRoutes,
  type RunTraceContext,
  type TracesResponse,
} from '../../shared/api'
import { RunStore } from '../log/run-store'
import { startTelemetry } from '../telemetry'
import { fixtureSpanId, fixtureTraces } from '../telemetry/fixtures'
import type { SpanLimits } from '../telemetry/span-store'
import { apiRouteTable } from './api-routes'

const TRACE = '4bf92f3577b34da6a3ce929d0e0e4736'
const PARENT = '00f067aa0ba902b7'
const cleanups: Array<() => Promise<unknown>> = []
afterEach(async () => {
  while (cleanups.length) await cleanups.pop()?.()
})

async function viewer(spanLimits?: SpanLimits) {
  const dir = await mkdtemp(join(tmpdir(), 'neo-viewer-traces-'))
  const store = await RunStore.open(dir)
  const telemetry = startTelemetry({
    hostApi: false,
    now: Date.now,
    spanLimits,
  })
  const api = apiRouteTable(
    store,
    async () => {
      throw new Error('unused')
    },
    async () => {
      throw new Error('unused')
    },
    async () => {},
    telemetry,
  )
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    routes: {
      [apiRoutes.traces]: api[apiRoutes.traces],
      ...telemetry.receiverRoutes,
    },
    fetch: () => new Response('not found', { status: 404 }),
  })
  const url = `http://127.0.0.1:${server.port}`
  cleanups.push(async () => {
    await server.stop(true)
    await telemetry.stop()
    await store.flush()
    await rm(dir, { recursive: true, force: true })
  })
  const search = async (query = ''): Promise<TracesResponse> => {
    const response = await fetch(`${url}${apiRoutes.traces}${query}`)
    expect(response.status).toBe(200)
    return response.json()
  }
  const push = async (
    neoRunId: string,
    at = 1_000,
    parent: string | null = null,
    ops?: number[],
  ) => {
    const body = fixtureTraces(
      { neoRunId, traceId: TRACE, parentSpanId: parent, startMs: at },
      { ops },
    )
    const response = await fetch(`${url}/otlp/v1/traces`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    })
    expect(response.status).toBe(200)
  }
  return { dir, store, url, search, push }
}

function record(
  store: RunStore,
  neoRunId: string | null,
  at = 1_000,
  trace: RunTraceContext | null = null,
) {
  const id = store.startRun({
    arrival: store.nextArrival(),
    tool: 'run',
    code: 'await page.goto("https://example.com")',
    timeoutMs: null,
    client: null,
    agentTaskId: null,
    trace,
    sentAt: new Date(at).toISOString(),
    proxyMs: 0,
    request: { raw: '{}', jsonRpcId: 1 },
  })
  if (neoRunId)
    store.endRun(id, {
      status: 'ok',
      answeredAt: new Date(at + 2_000).toISOString(),
      reply: {
        httpStatus: 200,
        contentType: 'application/json',
        contentEncoding: null,
        raw: '{}',
        text: '',
        structuredContent: null,
        isError: false,
        meta: { 'com.browseros.neo/run': { id: neoRunId } },
        rpcError: null,
        notifications: [],
        keepAlives: 0,
      },
    })
  return id
}

test('shared W3C ids produce current viewer run slices, not one merged trace or orphan telemetry', async () => {
  const v = await viewer()
  const first = record(v.store, 'first')
  const second = record(v.store, 'second', 2_000)
  await v.push('first')
  await v.push('second', 2_000)
  await v.push('not-in-proxy-log', 3_000)
  const result = await v.search('?traceId=4BF92F')
  expect(result.truncated).toBe(false)
  expect(result.traces.map((row) => row.run.id)).toEqual([second, first])
  expect(result.traces.map((row) => row.run.seq)).toEqual([2, 1])
  expect(result.traces[0]).toMatchObject({
    traceId: TRACE,
    spanCount: 9,
    errorCount: 1,
    durationMs: 1230,
    startedAt: 2_000,
    matchedSpan: null,
  })
  expect(await v.search('?traceId=ffffffff')).toEqual({
    traces: [],
    truncated: false,
    nextCursor: null,
  })
})

test('exact method and duration must match the same actual operation span', async () => {
  const v = await viewer()
  record(v.store, 'timing')
  await v.push('timing')
  expect((await v.search('?method=page.goto&maxMs=30')).traces).toEqual([])
  expect((await v.search('?method=page.goto&minMs=500')).traces).toEqual([])
  expect((await v.search('?method=page.got')).traces).toEqual([])
  expect((await v.search('?method=Page.goto')).traces).toEqual([])
  expect((await v.search('?method=page.goto&minMs=415')).traces).toEqual([])
  const exact = await v.search('?method=page.goto&minMs=414.999&maxMs=415')
  expect(exact.traces[0]?.matchedSpan).toEqual({
    spanId: fixtureSpanId('timing', 1),
    name: 'page.goto',
    label: 'page.goto',
    durationMs: 415,
  })
  const slow = await v.search('?method=locator.click&minMs=168&maxMs=508')
  expect(slow.traces[0]?.matchedSpan?.spanId).toBe(fixtureSpanId('timing', 7))
  expect(
    (await v.search('?minMs=500')).traces[0]?.matchedSpan?.durationMs,
  ).toBe(508)
})

test('canonical method matching retains raw labels and exact fractional duration boundaries at epoch timestamps', async () => {
  const v = await viewer()
  record(v.store, 'fractional')
  const body = fixtureTraces(
    {
      neoRunId: 'fractional',
      traceId: TRACE,
      parentSpanId: null,
      startMs: 1_763_650_000_000,
    },
    { ops: [0, 1] },
  )
  const op = body.resourceSpans[0]?.scopeSpans[0]?.spans[0]
  if (!op) throw new Error('missing fixture operation')
  op.name = 'unknown'
  op.startTimeUnixNano = '1763650000000000123'
  op.endTimeUnixNano = '1763650000500100123'
  await fetch(`${v.url}/otlp/v1/traces`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
  const result = await v.search('?method=unknown&minMs=500.099999&maxMs=500.1')
  expect(result.traces[0]?.matchedSpan).toMatchObject({
    name: 'unknown',
    label: 'page.goto',
  })
  expect(result.traces[0]?.matchedSpan?.durationMs).toBe(500.1)
  expect((await v.search('?method=unknown&minMs=500.1')).traces).toEqual([])
  expect((await v.search('?method=unknown&maxMs=500.099999')).traces).toEqual(
    [],
  )
  expect((await v.search('?method=page.goto')).traces).toEqual([])
})

test('missing spans never become matches from script text or an unrelated method', async () => {
  const v = await viewer()
  record(v.store, 'missing')
  record(v.store, 'partial')
  await v.push('partial', 1_000, null, [2])
  expect((await v.search()).traces).toHaveLength(1)
  expect((await v.search('?method=page.goto')).traces).toEqual([])
  expect(
    (await v.search('?method=locator.textContent')).traces[0],
  ).toMatchObject({
    spanCount: 1,
    durationMs: 25,
    matchedSpan: { label: 'locator.textContent', durationMs: 25 },
  })
})

test('only a viewer-minted parent binds a run without a reply; Clear drops every old search row', async () => {
  const v = await viewer()
  const minted = record(v.store, null, 1_000, {
    traceId: TRACE,
    spanId: PARENT,
    injected: true,
  })
  record(v.store, null, 2_000, {
    traceId: TRACE,
    spanId: PARENT,
    injected: false,
  })
  await v.push('no-reply', 1_000, PARENT)
  expect((await v.search()).traces.map((row) => row.run.id)).toEqual([minted])
  await v.store.clear()
  expect(await v.search()).toEqual({
    traces: [],
    truncated: false,
    nextCursor: null,
  })
  record(v.store, 'fresh', 4_000)
  expect((await v.search()).traces).toEqual([])
  await v.push('fresh', 4_000)
  expect((await v.search()).traces.map((row) => row.run.seq)).toEqual([1])
})

test('current bindings survive restart and late arrival renumbering', async () => {
  const v = await viewer()
  const later = record(v.store, 'later', 3_000)
  record(v.store, 'earlier', 1_000)
  await v.push('later', 3_000)
  expect((await v.search()).traces[0]?.run).toMatchObject({ id: later, seq: 2 })
  await v.store.flush()
  const reopened = await RunStore.open(v.dir)
  expect([...reopened.traceRuns('later', null)]).toEqual([
    { id: later, seq: 2, sentAt: new Date(3_000).toISOString(), status: 'ok' },
  ])
  await reopened.flush()
})

test('result limits report truncation and evicted spans disappear even when proxy runs remain', async () => {
  const v = await viewer({
    maxRuns: 2,
    maxSpansPerRun: 100,
    maxSpans: 100,
    maxBytes: 1e6,
  })
  for (const [index, name] of ['old', 'middle', 'new'].entries()) {
    record(v.store, name, (index + 1) * 1_000)
    await v.push(name, (index + 1) * 1_000)
  }
  expect(v.store.list()).toHaveLength(3)
  const limited = await v.search('?limit=1')
  expect(limited.truncated).toBe(true)
  expect(limited.traces[0]?.run.seq).toBe(3)
  const all = await v.search('?limit=2')
  expect(all.truncated).toBe(false)
  expect(all.traces.map((row) => row.run.seq)).toEqual([3, 2])
})

test('an exact shared trace id can continue through the storage cap despite new arrivals and late span starts', async () => {
  const v = await viewer()
  const ids: string[] = []
  for (let index = 0; index < 500; index++) {
    const at = (index + 1) * 1_000
    ids.push(record(v.store, `shared-${index}`, at))
    await v.push(`shared-${index}`, at, null, [0, 1])
  }
  const filters = `?traceId=${TRACE}&method=page.goto&minMs=414.999&maxMs=415`
  const first = await v.search(filters)
  expect(first.traces).toHaveLength(100)
  expect(first.nextCursor).not.toBeNull()

  // Neither a root's corrected start nor a newer arrival changes the immutable page boundary.
  await v.push('shared-499', 1, null, [0, 1])
  record(v.store, 'new-arrival', 600_000)
  await v.push('new-arrival', 600_000, null, [0, 1])
  const seen = [...first.traces]
  let cursor = first.nextCursor
  while (cursor) {
    const page = await v.search(`${filters}&cursor=${cursor}`)
    expect(page.traces.length).toBeLessThanOrEqual(100)
    seen.push(...page.traces)
    cursor = page.nextCursor
  }
  // The oldest span set was evicted, while the new arrival belongs to a later first-page refresh.
  expect(seen.map((row) => row.run.id)).toEqual(ids.slice(1).reverse())
  expect(new Set(seen.map((row) => row.run.id)).size).toBe(499)
  expect((await v.search(filters)).traces[0]?.run.seq).toBe(501)
})

test('query boundaries reject invalid values and a foreign host cannot read trace search', async () => {
  const v = await viewer()
  for (const query of [
    'traceId=xyz',
    `traceId=${'a'.repeat(33)}`,
    `method=${'a'.repeat(257)}`,
    'minMs=-1',
    'maxMs=Infinity',
    'minMs=NaN',
    'minMs=5&maxMs=5',
    'limit=0',
    'limit=201',
    'limit=1.5',
    'cursor=invalid',
    'cursor=e30',
    'cursor=WzEsMl0',
  ]) {
    const response = await fetch(`${v.url}${apiRoutes.traces}?${query}`)
    expect(response.status).toBe(400)
  }
  const foreign = await fetch(`${v.url}${apiRoutes.traces}`, {
    headers: { host: 'attacker.example' },
  })
  expect(foreign.status).toBe(403)
  const post = await fetch(`${v.url}${apiRoutes.traces}`, { method: 'POST' })
  expect(post.status).toBe(404)
})
