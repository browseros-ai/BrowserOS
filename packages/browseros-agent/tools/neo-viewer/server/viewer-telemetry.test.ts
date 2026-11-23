/**
 * Neo's telemetry through a running viewer, end to end: a run through the proxy, its spans and
 * metrics pushed to `/otlp/*`, Neo's host API scraped, and what the page reads back
 * (`/api/runs`, `/api/runs/:id/trace`, `/api/metrics`, `/api/events`). Neo is played by fakes.
 */

import { afterEach, describe, expect, spyOn, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  apiRoutes,
  type MetricsResponse,
  type RunResponse,
  type RunsResponse,
  type RunTraceResponse,
  type StreamEvent,
  type ViewerInfo,
} from '../shared/api'
import {
  FIXTURE_CODE,
  fixtureHostRun,
  fixtureMetrics,
  fixturePrometheus,
  fixtureTraces,
} from './telemetry/fixtures'
import { runResult, sseEvent, startFakeNeo } from './test-support/fake-neo'
import { FAKE_TOKEN, startFakeNeoHost } from './test-support/fake-neo-host'
import { type RunningViewer, startViewer } from './viewer-server'

/** The run id the fake Neo's replies carry (`runResult`). */
const NEO_RUN_ID = '01a1cc18-ed14-78b3-a30d-2cb4ef81d9f4'

const cleanups: Array<() => Promise<unknown>> = []
afterEach(async () => {
  while (cleanups.length > 0) await cleanups.pop()?.()
})

async function until<T>(
  read: () => Promise<T | null | undefined | false>,
  ms = 4_000,
) {
  const deadline = Date.now() + ms
  for (;;) {
    const value = await read()
    if (value) return value
    if (Date.now() > deadline) throw new Error('timed out')
    await Bun.sleep(10)
  }
}

/** A viewer in front of a fake Neo; with `host`, a fake host API answering as `respond` says. */
async function viewerWith(
  options: { host?: (path: string) => Response | undefined } = {},
) {
  const neo = startFakeNeo(() => ({
    chunks: [sseEvent(runResult(1, '"done"', { value: 'done' }))],
  }))
  cleanups.push(() => neo.stop())
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-telemetry-'))
  cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
  let hostApi = null
  if (options.host) {
    const host = startFakeNeoHost()
    const answer = options.host
    host.respond = (request) => answer(request.path.split('?')[0] ?? '')
    cleanups.push(() => host.stop())
    const tokenFile = join(dataDir, 'host-token')
    await writeFile(tokenFile, FAKE_TOKEN, { mode: 0o600 })
    hostApi = { url: new URL(host.url), tokenFile }
  }
  const viewer = await startViewer(
    { port: 0, upstream: new URL(neo.url), dataDir, hostApi },
    { telemetry: { scrape: { intervalMs: 20, retryMs: 20 }, emitEveryMs: 20 } },
  )
  cleanups.push(() => viewer.stop())
  return { neo, viewer, events: listen(viewer) }
}

/** Everything `/api/events` sends, as it arrives. */
function listen(viewer: RunningViewer): StreamEvent[] {
  const events: StreamEvent[] = []
  const reading = (async () => {
    const response = await fetch(`${viewer.url}${apiRoutes.events}`)
    const reader = response.body?.getReader()
    cleanups.push(async () => {
      await reader?.cancel()
    })
    let buffer = ''
    for (;;) {
      const chunk = await reader?.read().catch(() => null)
      if (!chunk || chunk.done) return
      buffer += new TextDecoder().decode(chunk.value)
      const blocks = buffer.split('\n\n')
      buffer = blocks.pop() ?? ''
      for (const block of blocks)
        if (block.startsWith('data: '))
          events.push(JSON.parse(block.slice(6)) as StreamEvent)
    }
  })()
  cleanups.push(() => reading)
  return events
}

async function get<T>(viewer: RunningViewer, path: string): Promise<T> {
  return (await (await fetch(`${viewer.url}${path}`)).json()) as T
}

function otlp(
  viewer: RunningViewer,
  signal: 'traces' | 'metrics',
  body: unknown,
) {
  return fetch(`${viewer.url}/otlp/v1/${signal}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body),
  })
}

/** One run through the proxy; what Neo got in its `traceparent` header. */
async function runThrough(
  viewer: RunningViewer,
  neo: { requests: Array<{ headers: Headers }> },
) {
  await fetch(viewer.mcpUrl, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/call',
      params: { name: 'run', arguments: { code: FIXTURE_CODE } },
    }),
  }).then((response) => response.text())
  const [, traceId = '', spanId = ''] =
    neo.requests.at(-1)?.headers.get('traceparent')?.split('-') ?? []
  const run = await until(async () => {
    const { runs } = await get<RunsResponse>(viewer, apiRoutes.runs)
    return runs.find((candidate) => candidate.status === 'ok')
  })
  return { run, traceId, spanId }
}

describe('no host API (--upstream): spans fill the op table', () => {
  test("a run's spans become its Neo record, live, with its trace; pushed metrics are its health", async () => {
    const { neo, viewer, events } = await viewerWith()
    const info = await get<ViewerInfo>(viewer, apiRoutes.info)
    expect(info.otlp).toEqual({
      endpoint: `${viewer.url}/otlp`,
      containerEndpoint: `http://host.docker.internal:${new URL(viewer.url).port}/otlp`,
    })
    const { run, traceId, spanId } = await runThrough(viewer, neo)
    expect(run.trace).toEqual({ traceId, spanId, injected: true })
    expect(run.operationCount).toBeNull()
    // Neo pushes the run's spans under the viewer's traceparent, then its metrics.
    const pushed = await otlp(
      viewer,
      'traces',
      fixtureTraces({
        neoRunId: NEO_RUN_ID,
        traceId,
        parentSpanId: spanId,
        startMs: Date.now(),
      }),
    )
    expect(pushed.status).toBe(200)
    expect((await otlp(viewer, 'metrics', fixtureMetrics(0))).status).toBe(200)

    const listed = await until(async () => {
      const { runs } = await get<RunsResponse>(viewer, apiRoutes.runs)
      return runs[0]?.operationCount === 8 ? runs[0] : null
    })
    expect(listed.neoDurationMs).toBe(1230)
    const { run: detail } = await get<RunResponse>(
      viewer,
      apiRoutes.run(run.id),
    )
    expect(detail.neo).toMatchObject({
      source: 'spans',
      complete: true,
      status: 'ok',
    })
    expect(detail.neo?.operations.map((op) => op.site?.line ?? null)).toEqual([
      1,
      2,
      3,
      4,
      null,
      5,
      6,
      7,
    ])

    const { trace } = await get<RunTraceResponse>(
      viewer,
      apiRoutes.runTrace(run.id),
    )
    expect(trace).toMatchObject({ traceId, complete: true })
    expect(trace?.spans).toHaveLength(9)
    expect(trace?.spans[0]).toMatchObject({
      type: 'run',
      name: 'neo.run',
      parentSpanId: spanId,
    })

    const { metrics } = await get<MetricsResponse>(viewer, apiRoutes.metrics)
    expect(metrics).toMatchObject({ source: 'push', runs: { total: 4 } })
    const snapshot = await get<RunsResponse>(viewer, apiRoutes.runs)
    expect(snapshot.health).toMatchObject({
      source: 'push',
      spansSeen: true,
      tabsOpen: 3,
    })

    // The page heard it all on the one stream: the run's record as a run change, health apart.
    await until(async () => events.some((event) => event.type === 'health'))
    const health = events.find((event) => event.type === 'health')
    expect(health && 'version' in health).toBe(false)
    expect(
      events.some(
        (event) => event.type === 'run' && event.run.operationCount === 8,
      ),
    ).toBe(true)
    // An unknown run has no trace; a run with no spans has a null one.
    expect(
      (await fetch(`${viewer.url}${apiRoutes.runTrace('nope')}`)).status,
    ).toBe(404)
  })

  test('an image that never pushes: the run keeps no record, as before, and nothing is said', async () => {
    const said = [
      spyOn(console, 'error'),
      spyOn(console, 'warn'),
      spyOn(console, 'log'),
    ]
    const { neo, viewer } = await viewerWith()
    const { run } = await runThrough(viewer, neo)
    // Copy waits for a record that never comes: at most the first-span grace (2 s).
    const started = Date.now()
    const { run: detail } = await get<RunResponse>(
      viewer,
      apiRoutes.run(run.id, { waitForNeo: true }),
    )
    expect(Date.now() - started).toBeLessThan(3_000)
    expect(detail.neo).toBeNull()
    expect(detail.operationCount).toBeNull()
    expect(
      (await get<RunTraceResponse>(viewer, apiRoutes.runTrace(run.id))).trace,
    ).toBeNull()
    expect(
      (await get<MetricsResponse>(viewer, apiRoutes.metrics)).metrics,
    ).toBeNull()
    expect((await get<RunsResponse>(viewer, apiRoutes.runs)).health).toBeNull()
    for (const spy of said) {
      expect(spy).not.toHaveBeenCalled()
      spy.mockRestore()
    }
  })
})

describe('with a host API: /v1/runs owns the op table, spans are the trace', () => {
  test("Neo's own record stays the run's, spans never replace it; the scrape is the health", async () => {
    const { neo, viewer } = await viewerWith({
      host: (path) => {
        if (path === `/v1/runs/${NEO_RUN_ID}`)
          return Response.json(
            fixtureHostRun({
              neoRunId: NEO_RUN_ID,
              traceId: '',
              parentSpanId: null,
              startMs: 1_000_000,
            }),
          )
        if (path === '/v1/metrics')
          return new Response(fixturePrometheus(1), {
            headers: {
              'content-type': 'text/plain; version=0.0.4; charset=utf-8',
            },
          })
        return undefined
      },
    })
    const { run, traceId, spanId } = await runThrough(viewer, neo)
    const joined = await until(async () => {
      const { run: detail } = await get<RunResponse>(
        viewer,
        apiRoutes.run(run.id),
      )
      return detail.neo ? detail : null
    })
    expect(joined.neo?.source).toBeUndefined()
    expect(joined.neo?.operations[0]?.args).toEqual({
      url: 'https://example.com',
    })
    // Spans arrive after: the waterfall has them, the op table is still Neo's record.
    await otlp(
      viewer,
      'traces',
      fixtureTraces({
        neoRunId: NEO_RUN_ID,
        traceId,
        parentSpanId: spanId,
        startMs: Date.now(),
      }),
    )
    await otlp(viewer, 'metrics', fixtureMetrics(9))
    const { trace } = await get<RunTraceResponse>(
      viewer,
      apiRoutes.runTrace(run.id),
    )
    expect(trace?.spans).toHaveLength(9)
    await Bun.sleep(50)
    const { run: after } = await get<RunResponse>(viewer, apiRoutes.run(run.id))
    expect(after.neo).toEqual(joined.neo)
    // Both doors live: the scrape's numbers (5 runs), not the push's (13).
    const { metrics } = await until(async () => {
      const answer = await get<MetricsResponse>(viewer, apiRoutes.metrics)
      return answer.metrics?.source === 'scrape' ? answer : null
    })
    expect(metrics?.runs.total).toBe(5)
  })

  test('an image older than telemetry: /v1/metrics answers 404, health stays empty, silently', async () => {
    const said = [
      spyOn(console, 'error'),
      spyOn(console, 'warn'),
      spyOn(console, 'log'),
    ]
    const { neo, viewer } = await viewerWith({
      host: (path) =>
        path === `/v1/runs/${NEO_RUN_ID}`
          ? Response.json(
              fixtureHostRun({
                neoRunId: NEO_RUN_ID,
                traceId: '',
                parentSpanId: null,
                startMs: 1,
              }),
            )
          : undefined,
    })
    const { run } = await runThrough(viewer, neo)
    await until(async () => {
      const { run: detail } = await get<RunResponse>(
        viewer,
        apiRoutes.run(run.id),
      )
      return detail.operationCount === 8
    })
    await Bun.sleep(100)
    expect(
      (await get<MetricsResponse>(viewer, apiRoutes.metrics)).metrics,
    ).toBeNull()
    expect((await get<RunsResponse>(viewer, apiRoutes.runs)).health).toBeNull()
    expect(
      (await get<RunTraceResponse>(viewer, apiRoutes.runTrace(run.id))).trace,
    ).toBeNull()
    for (const spy of said) {
      expect(spy).not.toHaveBeenCalled()
      spy.mockRestore()
    }
  })
})
