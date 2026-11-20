/**
 * The JSON API the web UI reads (`shared/api.ts` names the routes and shapes), served from the
 * in-memory `RunStore` and Neo's telemetry (`server/telemetry/`). Requests must name this machine
 * in `Host`: the viewer listens on 127.0.0.1 only, and refusing other host names keeps a web page
 * that rebinds its own domain to 127.0.0.1 from reading the log. Clear and Export must also come
 * from the viewer's own page (`Origin`), so a page elsewhere cannot empty the log or fill the disk
 * with a form post. (Neo's OTLP push has its own routes and guard: `telemetry/otlp-receiver.ts`.)
 */

import type {
  ExportResponse,
  MetricsResponse,
  RunResponse,
  RunsResponse,
  RunTraceResponse,
  StreamEvent,
  ViewerInfo,
} from '../../shared/api'
import { apiRoutes } from '../../shared/api'
import type { RunStore } from '../log/run-store'
import type { Telemetry } from '../telemetry'

const LOCAL_HOSTS = new Set(['127.0.0.1', 'localhost', '[::1]'])
const EVENT_KEEP_ALIVE_MS = 15_000

type RouteRequest<P extends string> = Request & { params: Record<P, string> }

export function apiRouteTable(
  store: RunStore,
  info: () => Promise<ViewerInfo>,
  exportRuns: () => Promise<ExportResponse>,
  /** Waits, bounded, for Neo's record of a run that just ended (`?wait=neo`). */
  awaitNeoRecord: (runId: string) => Promise<void>,
  telemetry: Pick<Telemetry, 'health' | 'subscribe' | 'metrics' | 'runTrace'>,
) {
  const snapshot = (): RunsResponse => ({
    log: store.summary(),
    runs: store.list(),
    version: store.version,
    health: telemetry.health(),
  })
  return {
    [apiRoutes.info]: local(async () =>
      Response.json((await info()) satisfies ViewerInfo),
    ),
    [apiRoutes.runs]: local(() => Response.json(snapshot())),
    '/api/runs/:runId': local(async (req: RouteRequest<'runId'>) => {
      if (new URL(req.url).searchParams.get('wait') === 'neo')
        await awaitNeoRecord(req.params.runId)
      const run = store.run(req.params.runId)
      return run
        ? Response.json({ run } satisfies RunResponse)
        : notFound('run')
    }),
    // A run's spans, found by Neo's run id (or the span id the viewer minted for a run with no
    // reply), never by trace id alone: a caller may share one trace across many runs.
    '/api/runs/:runId/trace': local((req: RouteRequest<'runId'>) => {
      const run = store.run(req.params.runId)
      if (!run) return notFound('run')
      const trace = telemetry.runTrace({
        neoRunId: run.neoRunId,
        mintedSpanId: run.trace?.injected ? run.trace.spanId : null,
      })
      return Response.json({ trace } satisfies RunTraceResponse)
    }),
    [apiRoutes.metrics]: local(() =>
      Response.json({ metrics: telemetry.metrics() } satisfies MetricsResponse),
    ),
    [apiRoutes.clear]: {
      POST: local(
        fromOwnPage(async () => {
          await store.clear()
          return Response.json(snapshot())
        }),
      ),
    },
    [apiRoutes.export]: {
      POST: local(
        fromOwnPage(async () => {
          try {
            return Response.json((await exportRuns()) satisfies ExportResponse)
          } catch (error) {
            return Response.json(
              { error: `export failed: ${String(error)}` },
              { status: 500 },
            )
          }
        }),
      ),
    },
    [apiRoutes.events]: local(() => eventStream(store, telemetry)),
  }
}

/** Whether a request's `Host` names this machine. */
export function isLocalRequest(req: Request): boolean {
  const host = req.headers.get('host') ?? ''
  const name = host.startsWith('[')
    ? host.slice(0, host.indexOf(']') + 1)
    : host.split(':')[0]
  return LOCAL_HOSTS.has(name ?? '')
}

function local<R extends Request>(
  handler: (req: R) => Response | Promise<Response>,
): (req: R) => Response | Promise<Response> {
  return (req) =>
    isLocalRequest(req)
      ? handler(req)
      : new Response('forbidden host\n', { status: 403 })
}

/** Refuses a request whose `Origin` is not the viewer's own page. */
function fromOwnPage(
  handler: (req: Request) => Promise<Response>,
): (req: Request) => Promise<Response> {
  return async (req) =>
    req.headers.get('origin') === `http://${req.headers.get('host')}`
      ? handler(req)
      : new Response('forbidden origin\n', { status: 403 })
}

function notFound(what: string): Response {
  return Response.json({ error: `no such ${what}` }, { status: 404 })
}

/**
 * Every store change, and every health change, as one SSE `data:` line each (`StreamEvent`);
 * comments keep idle connections open. Health has no `version`: it is not a change of the list.
 */
function eventStream(
  store: RunStore,
  telemetry: Pick<Telemetry, 'subscribe'>,
): Response {
  const encoder = new TextEncoder()
  let stop = () => {}
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      const send = (text: string) => {
        try {
          controller.enqueue(encoder.encode(text))
        } catch {
          stop()
        }
      }
      const event = (message: StreamEvent) =>
        send(`data: ${JSON.stringify(message)}\n\n`)
      const unsubscribe = store.subscribe(event)
      const unsubscribeHealth = telemetry.subscribe((health) =>
        event({ type: 'health', health }),
      )
      const timer = setInterval(
        () => send(': keep-alive\n\n'),
        EVENT_KEEP_ALIVE_MS,
      )
      stop = () => {
        unsubscribe()
        unsubscribeHealth()
        clearInterval(timer)
      }
      send(': connected\n\n')
    },
    cancel() {
      stop()
    },
  })
  return new Response(stream, {
    headers: {
      'content-type': 'text/event-stream',
      'cache-control': 'no-cache',
    },
  })
}
