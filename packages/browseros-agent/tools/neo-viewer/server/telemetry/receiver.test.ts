import { afterEach, describe, expect, test } from 'bun:test'
import { startTelemetry, type Telemetry } from '.'
import { fixtureMetrics, fixtureTraces } from './fixtures'

const RUN = {
  neoRunId: 'r1',
  traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
  parentSpanId: null,
  startMs: 1_000_000,
}

const cleanups: Array<() => Promise<unknown>> = []
afterEach(async () => {
  while (cleanups.length > 0) await cleanups.pop()?.()
})

/** The receiver on a real loopback server, as the viewer mounts it. */
function serve(): { url: string; telemetry: Telemetry } {
  const telemetry = startTelemetry({ hostApi: false, now: Date.now })
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    routes: telemetry.receiverRoutes,
    fetch: () => new Response('not found\n', { status: 404 }),
  })
  cleanups.push(async () => {
    await telemetry.stop()
    await server.stop(true)
  })
  return { url: `http://127.0.0.1:${server.port}`, telemetry }
}

function post(
  url: string,
  body: string | Uint8Array,
  headers: Record<string, string> = { 'content-type': 'application/json' },
) {
  return fetch(url, { method: 'POST', headers, body: body as BodyInit })
}

describe('OTLP/HTTP JSON in', () => {
  test('traces, plain and gzip: accepted with an empty response, filed under their run', async () => {
    const { url, telemetry } = serve()
    const plain = await post(
      `${url}/otlp/v1/traces`,
      JSON.stringify(fixtureTraces(RUN, { ops: [1, 2] })),
    )
    expect(plain.status).toBe(200)
    expect(await plain.json()).toEqual({})
    const gzip = await post(
      `${url}/otlp/v1/traces`,
      Bun.gzipSync(JSON.stringify(fixtureTraces(RUN, { ops: [0] }))),
      { 'content-type': 'application/json', 'content-encoding': 'gzip' },
    )
    expect(gzip.status).toBe(200)
    expect(
      telemetry.runTrace({ neoRunId: 'r1', mintedSpanId: null }),
    ).toMatchObject({ complete: true, spans: [{ name: 'neo.run' }, {}, {}] })
    expect(telemetry.health()).toMatchObject({ spansSeen: true, source: null })
  })

  test('metrics, plain and gzip: the push becomes the shown source', async () => {
    const { url, telemetry } = serve()
    expect(telemetry.metrics()).toBeNull()
    for (const [body, headers] of [
      [
        JSON.stringify(fixtureMetrics(0)),
        { 'content-type': 'application/json' },
      ],
      [
        Bun.gzipSync(JSON.stringify(fixtureMetrics(1))),
        {
          'content-type': 'application/json; charset=utf-8',
          'content-encoding': 'gzip',
        },
      ],
    ] as const) {
      const response = await post(`${url}/otlp/v1/metrics`, body, {
        ...headers,
      })
      expect(response.status).toBe(200)
    }
    expect(telemetry.metrics()).toMatchObject({
      source: 'push',
      runs: { total: 5, inFlight: 1 },
    })
    expect(telemetry.health()).toMatchObject({ source: 'push', tabsOpen: 3 })
  })

  test('spans it cannot file come back as a partial success', async () => {
    const { url } = serve()
    const traces = fixtureTraces(RUN, { ops: [1] })
    traces.resourceSpans[0]?.scopeSpans[0]?.spans.push({
      ...(traces.resourceSpans[0]?.scopeSpans[0]?.spans[0] as object),
      name: 'http.request',
      attributes: [],
    } as never)
    const response = await post(`${url}/otlp/v1/traces`, JSON.stringify(traces))
    expect(response.status).toBe(200)
    expect(await response.json()).toMatchObject({
      partialSuccess: { rejectedSpans: '1' },
    })
  })
})

describe('what the receiver refuses', () => {
  test('malformed JSON, and JSON that is not an export request: 400', async () => {
    const { url } = serve()
    const broken = await post(`${url}/otlp/v1/traces`, '{"resourceSpans": [')
    expect(broken.status).toBe(400)
    expect(((await broken.json()) as { message: string }).message).toContain(
      'malformed OTLP JSON',
    )
    const wrong = await post(`${url}/otlp/v1/metrics`, '{"resourceMetrics": 5}')
    expect(wrong.status).toBe(400)
    const badGzip = await post(`${url}/otlp/v1/traces`, 'not gzip', {
      'content-type': 'application/json',
      'content-encoding': 'gzip',
    })
    expect(badGzip.status).toBe(400)
  })

  test('protobuf gets a 415 that names the setting to change; other types and codings too', async () => {
    const { url } = serve()
    const protobuf = await post(
      `${url}/otlp/v1/traces`,
      new Uint8Array([0x0a, 0x00]),
      { 'content-type': 'application/x-protobuf' },
    )
    expect(protobuf.status).toBe(415)
    expect(((await protobuf.json()) as { message: string }).message).toContain(
      'OTEL_EXPORTER_OTLP_PROTOCOL=http/json',
    )
    const refused: Array<Record<string, string>> = [
      { 'content-type': 'text/plain' },
      {},
      { 'content-type': 'application/json', 'content-encoding': 'br' },
    ]
    for (const headers of refused) {
      const response = await post(`${url}/otlp/v1/metrics`, '{}', headers)
      expect(response.status).toBe(415)
    }
  })

  test('over 4 MiB, sent whole or gzipped small: 413, and the body is never inflated past the cap', async () => {
    const { url } = serve()
    const big = `{"resourceSpans":[],"pad":"${'x'.repeat(5 * 1024 * 1024)}"}`
    expect((await post(`${url}/otlp/v1/traces`, big)).status).toBe(413)
    const bomb = Bun.gzipSync(big)
    expect(bomb.length).toBeLessThan(64 * 1024)
    const inflated = await post(`${url}/otlp/v1/traces`, bomb, {
      'content-type': 'application/json',
      'content-encoding': 'gzip',
    })
    expect(inflated.status).toBe(413)
  })

  test('a browser (any Origin) or a rebound domain is refused; a container naming host.docker.internal is not', async () => {
    const { url } = serve()
    const body = JSON.stringify(fixtureMetrics(0))
    const json = { 'content-type': 'application/json' }
    expect(
      (await post(`${url}/otlp/v1/metrics`, body, { ...json, origin: url }))
        .status,
    ).toBe(403)
    expect(
      (
        await post(`${url}/otlp/v1/metrics`, body, {
          ...json,
          host: 'attacker.example',
        })
      ).status,
    ).toBe(403)
    const port = new URL(url).port
    for (const host of [
      `host.docker.internal:${port}`,
      `localhost:${port}`,
      `[::1]:${port}`,
    ]) {
      const response = await post(`${url}/otlp/v1/metrics`, body, {
        ...json,
        host,
      })
      expect(response.status).toBe(200)
    }
  })
})
