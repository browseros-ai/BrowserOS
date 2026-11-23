/**
 * Neo's telemetry, played, so the page can be built before Neo emits any (wire-contract.md).
 *
 *   bun server/telemetry/feed-fixture.ts serve [--port 7392]
 *       A stand-in Neo: MCP at /mcp (every `run` call answers `ok` with a fresh run id) and a
 *       host API (/v1/runs/{id}, /v1/metrics; bearer token in the file it prints). It prints the
 *       two ways to start a viewer against it: spans mode (no host API) and host mode.
 *
 *   bun server/telemetry/feed-fixture.ts feed --viewer http://127.0.0.1:7393 [--seconds 10] [--gzip]
 *       One run through the viewer's MCP URL (the fixture's 8-line script), then, as Neo would,
 *       its spans (neo.run + 8 operations, one failed, one nested) and the 16 metrics to the viewer's
 *       /otlp, the metrics again every 500 ms for --seconds. `--run <viewer run id>` feeds spans
 *       for a run already in the list instead of making one.
 *
 * Against a real Neo the script really runs (it opens example.com): point the viewer at `serve`.
 */

import { randomBytes, randomUUID } from 'node:crypto'
import { mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { parseArgs } from 'node:util'
import type { RunSummary, RunsResponse } from '../../shared/api'
import {
  FIXTURE_CODE,
  fixtureHostRun,
  fixtureMetrics,
  fixturePrometheus,
  fixtureTraces,
} from './fixtures'

const RUN_META = 'com.browseros.neo/run'

export interface StandIn {
  /** `http://127.0.0.1:<port>`: MCP at `/mcp`, the host API at `/v1/*`. */
  url: string
  tokenFile: string
  stop(): Promise<void>
}

/** A Neo that answers every run `ok`, and serves the fixture's record and metrics. */
export async function serveStandIn(port = 0): Promise<StandIn> {
  const token = randomBytes(32).toString('hex')
  const tokenFile = join(
    await mkdtemp(join(tmpdir(), 'neo-fixture-')),
    'host-token',
  )
  await writeFile(tokenFile, token, { mode: 0o600 })
  const runs = new Map<string, number>()
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port,
    async fetch(req) {
      const { pathname } = new URL(req.url)
      if (pathname === '/mcp') return mcp(req, runs)
      if (req.headers.get('authorization') !== `Bearer ${token}`)
        return Response.json(
          { code: 'unauthorized', message: 'missing bearer token' },
          { status: 401 },
        )
      if (pathname === '/v1/metrics')
        return new Response(fixturePrometheus(runs.size), {
          headers: {
            'content-type': 'text/plain; version=0.0.4; charset=utf-8',
          },
        })
      const id = /^\/v1\/runs\/([^/]+)$/.exec(pathname)?.[1]
      const startMs = id ? runs.get(decodeURIComponent(id)) : undefined
      if (id && startMs !== undefined)
        return Response.json(
          fixtureHostRun({
            neoRunId: decodeURIComponent(id),
            traceId: '',
            parentSpanId: null,
            startMs,
          }),
        )
      return Response.json(
        { code: 'not_found', message: 'nothing has this id' },
        { status: 404 },
      )
    },
  })
  return {
    url: `http://127.0.0.1:${server.port}`,
    tokenFile,
    stop: () => server.stop(true),
  }
}

/** Streamable-HTTP MCP, the little a viewer's agent needs: initialize, tools/list, run. */
async function mcp(req: Request, runs: Map<string, number>): Promise<Response> {
  const message = (await req.json().catch(() => null)) as {
    id?: unknown
    method?: string
  } | null
  if (!message || message.id === undefined)
    return new Response(null, { status: 202 })
  let result: unknown
  if (message.method === 'initialize')
    result = {
      protocolVersion: '2025-06-18',
      capabilities: { tools: {} },
      serverInfo: { name: 'neo-fixture', version: '1' },
    }
  else if (message.method === 'tools/list')
    result = {
      tools: [
        {
          name: 'run',
          description: 'Runs a script (stand-in).',
          inputSchema: { type: 'object' },
        },
      ],
    }
  else if (message.method === 'tools/call') {
    const neoRunId = randomUUID()
    runs.set(neoRunId, Date.now())
    result = {
      content: [
        {
          type: 'text',
          text: '{"heading":"Example Domain","links":1,"title":"Example Domain"}',
        },
      ],
      structuredContent: {
        value: { heading: 'Example Domain', links: 1, title: 'Example Domain' },
      },
      isError: false,
      _meta: {
        [RUN_META]: {
          id: neoRunId,
          status: 'ok',
          effects: 'done',
          unknown: [],
          tasks: [],
        },
      },
    }
  } else
    return sse({
      jsonrpc: '2.0',
      id: message.id,
      error: { code: -32601, message: 'method not found' },
    })
  return sse({ jsonrpc: '2.0', id: message.id, result })
}

function sse(message: unknown): Response {
  return new Response(`data: ${JSON.stringify(message)}\n\n`, {
    headers: { 'content-type': 'text/event-stream' },
  })
}

export interface FeedOptions {
  /** The viewer's base URL. */
  viewer: string
  /** Feed spans for this run in the viewer's list instead of making one. */
  runId?: string
  /** Keep pushing metrics (every 500 ms) for this long. */
  seconds?: number
  gzip?: boolean
  say?: (line: string) => void
}

/** What a feed did: the run it fed and what the viewer said to each push. */
export interface Fed {
  run: RunSummary
  neoRunId: string
  traces: number
  metricsPushes: number
}

export async function feed(options: FeedOptions): Promise<Fed> {
  const say = options.say ?? (() => {})
  const base = options.viewer.replace(/\/$/, '')
  const run = options.runId
    ? await runById(base, options.runId)
    : await makeRun(base)
  say(
    `run #${run.seq} (${run.id}), Neo run ${run.neoRunId ?? '–'}, trace ${run.trace?.traceId ?? '–'}`,
  )
  // A run whose reply never came still binds by the span id the viewer minted for it.
  const neoRunId = run.neoRunId ?? randomUUID()
  const traces = await push(
    base,
    'traces',
    options.gzip,
    fixtureTraces({
      neoRunId,
      traceId: run.trace?.traceId ?? randomBytes(16).toString('hex'),
      parentSpanId: run.trace?.spanId ?? null,
      startMs: Date.parse(run.sentAt) + run.proxyMs,
    }),
  )
  say(`spans: HTTP ${traces}`)
  let metricsPushes = 0
  const until = Date.now() + (options.seconds ?? 0) * 1000
  for (let step = 0; ; step++) {
    const status = await push(
      base,
      'metrics',
      options.gzip,
      fixtureMetrics(step),
    )
    metricsPushes++
    if (step === 0) say(`metrics: HTTP ${status}`)
    if (Date.now() >= until) break
    await Bun.sleep(500)
  }
  if (metricsPushes > 1) say(`metrics pushed ${metricsPushes} times`)
  return { run, neoRunId, traces, metricsPushes }
}

/** Sends the fixture's script through the viewer's MCP URL and waits for the run in its list. */
async function makeRun(base: string): Promise<RunSummary> {
  const reply = await fetch(`${base}/mcp`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: randomUUID(),
      method: 'tools/call',
      params: {
        name: 'run',
        arguments: { code: FIXTURE_CODE },
        _meta: {
          'io.modelcontextprotocol/clientInfo': {
            name: 'neo-fixture',
            version: '1',
          },
        },
      },
    }),
  }).then((response) => response.text())
  const neoRunId = replyRunId(reply)
  for (let attempt = 0; attempt < 100; attempt++) {
    const { runs } = (await (
      await fetch(`${base}/api/runs`)
    ).json()) as RunsResponse
    const found = [...runs]
      .reverse()
      .find(
        (run) =>
          run.code === FIXTURE_CODE &&
          run.status !== 'running' &&
          (!neoRunId || run.neoRunId === neoRunId),
      )
    if (found) return found
    await Bun.sleep(50)
  }
  throw new Error(`neo-fixture: the run did not reach ${base}/api/runs`)
}

/** Neo's run id from a reply (SSE `data:` lines or plain JSON): `result._meta[…].id`. */
function replyRunId(reply: string): string | null {
  const bodies = reply.includes('data:')
    ? reply
        .split('\n')
        .filter((line) => line.startsWith('data:'))
        .map((line) => line.slice(5))
    : [reply]
  for (const body of bodies) {
    try {
      const id = JSON.parse(body)?.result?._meta?.[RUN_META]?.id
      if (typeof id === 'string') return id
    } catch {
      // a line that is not the reply
    }
  }
  return null
}

async function runById(base: string, runId: string): Promise<RunSummary> {
  const response = await fetch(`${base}/api/runs/${encodeURIComponent(runId)}`)
  if (!response.ok) throw new Error(`neo-fixture: no run ${runId} at ${base}`)
  return ((await response.json()) as { run: RunSummary }).run
}

async function push(
  base: string,
  signal: 'traces' | 'metrics',
  gzip: boolean | undefined,
  body: unknown,
): Promise<number> {
  const json = JSON.stringify(body)
  const response = await fetch(`${base}/otlp/v1/${signal}`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      ...(gzip ? { 'content-encoding': 'gzip' } : {}),
    },
    body: gzip ? Bun.gzipSync(json) : json,
  })
  await response.body?.cancel()
  return response.status
}

if (import.meta.main) {
  const { values, positionals } = parseArgs({
    args: process.argv.slice(2),
    allowPositionals: true,
    options: {
      port: { type: 'string' },
      viewer: { type: 'string' },
      run: { type: 'string' },
      seconds: { type: 'string' },
      gzip: { type: 'boolean' },
    },
  })
  const command = positionals[0] ?? 'feed'
  if (command === 'serve') {
    const standIn = await serveStandIn(Number(values.port ?? 7392))
    console.log(`Stand-in Neo at ${standIn.url} (MCP ${standIn.url}/mcp). Start a viewer against it:
  spans mode (no host API):
    bun run neo:viewer -- --name nt-viewer-web --port 7393 --upstream ${standIn.url}/mcp
  host mode (/v1/runs owns the op table, /v1/metrics scraped):
    bun run neo:viewer -- --name nt-viewer-web --port 7393 --upstream ${standIn.url}/mcp --host-api ${standIn.url} --host-token-file ${standIn.tokenFile}
then: bun server/telemetry/feed-fixture.ts feed --viewer http://127.0.0.1:7393 --seconds 10
Ctrl-C stops it.`)
  } else if (command === 'feed') {
    if (!values.viewer)
      throw new Error('feed needs --viewer http://127.0.0.1:<port>')
    await feed({
      viewer: values.viewer,
      ...(values.run ? { runId: values.run } : {}),
      seconds: Number(values.seconds ?? 0),
      gzip: values.gzip ?? false,
      say: (line) => console.log(line),
    })
  } else throw new Error(`unknown command ${command}: serve or feed`)
}
