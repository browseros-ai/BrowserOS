/**
 * The JSON API the web UI reads (`shared/api.ts` names the routes and shapes), served from the
 * in-memory `RunStore`. Requests must name this machine in `Host`: the viewer listens on
 * 127.0.0.1 only, and refusing other host names keeps a web page that rebinds its own domain to
 * 127.0.0.1 from reading the log. Clear and Export must also come from the viewer's own page
 * (`Origin`), so a page elsewhere cannot empty the log or fill the disk with a form post.
 */

import type {
  ExportResponse,
  RunResponse,
  RunsResponse,
  ViewerInfo,
} from '../../shared/api'
import { apiRoutes } from '../../shared/api'
import type { RunStore } from '../log/run-store'

const LOCAL_HOSTS = new Set(['127.0.0.1', 'localhost', '[::1]'])
const EVENT_KEEP_ALIVE_MS = 15_000

type RouteRequest<P extends string> = Request & { params: Record<P, string> }

export function apiRouteTable(
  store: RunStore,
  info: () => Promise<ViewerInfo>,
  exportRuns: () => Promise<ExportResponse>,
) {
  const snapshot = (): RunsResponse => ({
    log: store.summary(),
    runs: store.list(),
    version: store.version,
  })
  return {
    [apiRoutes.info]: local(async () =>
      Response.json((await info()) satisfies ViewerInfo),
    ),
    [apiRoutes.runs]: local(() => Response.json(snapshot())),
    '/api/runs/:runId': local((req: RouteRequest<'runId'>) => {
      const run = store.run(req.params.runId)
      return run
        ? Response.json({ run } satisfies RunResponse)
        : notFound('run')
    }),
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
    [apiRoutes.events]: local(() => eventStream(store)),
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

/** Every store change as one SSE `data:` line; comments keep idle connections open. */
function eventStream(store: RunStore): Response {
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
      const unsubscribe = store.subscribe((event) =>
        send(`data: ${JSON.stringify(event)}\n\n`),
      )
      const timer = setInterval(
        () => send(': keep-alive\n\n'),
        EVENT_KEEP_ALIVE_MS,
      )
      stop = () => {
        unsubscribe()
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
