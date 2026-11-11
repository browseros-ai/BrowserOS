/**
 * The live screen's routes on the viewer (`shared/live-screen.ts` names them): the display
 * relay, the panel's status, and Take over / Give back over Neo's takeover flow.
 *
 * Who may call them: like the rest of `/api`, the request must name this machine in `Host`. The
 * display and the two POSTs also require an `Origin` of the viewer's own page. A WebSocket is
 * not covered by the same-origin policy and a form POST needs no preflight, so without that
 * check any web page the owner has open could drive the browser through `ws://127.0.0.1:7310`.
 *
 * Takeovers: `POST /v1/takeovers` locks the agent out of one tab until the takeover is released
 * or its TTL (20 min) runs out. The viewer remembers the takeovers it started, so a reloaded
 * page finds the one it holds, and gives them all back when the viewer stops, including one
 * whose start is still on its way to Neo.
 */

import type { Server } from 'bun'
import {
  displayQuery,
  type HostState,
  type LiveError,
  type LiveStatus,
  type LiveTab,
  type LiveTakeover,
  liveRoutes,
  parseDisplayQuery,
  type ScreenSize,
  type TakeoverRequest,
  type TakeoverResponse,
} from '../../shared/live-screen'
import { isLocalRequest } from '../api/api-routes'
import { DisplayRelay } from './display-relay'
import type { LiveHost } from './live-host'
import type { SocketSession } from './socket-session'

/** Operations read back from the task's trace to find the tab it is on. */
const TRACE_TAIL = 200
/**
 * Operations that only subscribe, so they make no tab the task's: Neo's own list
 * (`neo-trace` `SUBSCRIPTIONS`), which its `followTask` skips too.
 */
const SUBSCRIPTIONS = new Set([
  'events.listen',
  'events.interceptFileChooser',
  'events.next',
])
/** How long stopping the viewer waits for Neo to give back each held takeover. */
const RELEASE_ON_STOP_MS = 3_000

type RouteRequest<P extends string> = Request & { params: Record<P, string> }
type Handler<R extends Request> = (
  req: R,
  server: Server<SocketSession>,
) => Response | Promise<Response> | undefined

export type LiveScreen = ReturnType<typeof liveScreen>

export function liveScreen(host: LiveHost) {
  const held = new Map<string, LiveTakeover>()
  // Takeover starts on their way to Neo, which `stop` waits for before it gives back.
  const starting = new Set<Promise<Response>>()
  let stopping = false

  const routes = {
    [liveRoutes.display]: guarded({ sameOrigin: true }, (req, server) => {
      const request = parseDisplayQuery(new URL(req.url).searchParams)
      if ('error' in request)
        return failure(400, 'invalid_request', request.error)
      const relay = new DisplayRelay(
        host,
        `/v1/display?${displayQuery(request)}`,
      )
      // Neo echoes `binary` when a viewer offers it; the relay does the same for the page.
      const offered = req.headers.get('sec-websocket-protocol') ?? ''
      const headers = offered.split(',').some((p) => p.trim() === 'binary')
        ? { 'sec-websocket-protocol': 'binary' }
        : undefined
      if (server.upgrade(req, { data: relay, headers })) return undefined
      return failure(426, 'invalid_request', 'expected a WebSocket upgrade')
    }),

    [liveRoutes.status]: guarded({ sameOrigin: false }, async (req) => {
      const task = new URL(req.url).searchParams.get('task') || null
      return Response.json(
        (await status(host, held, task)) satisfies LiveStatus,
      )
    }),

    [liveRoutes.takeovers]: {
      POST: guarded({ sameOrigin: true }, async (req) => {
        const body = (await req
          .json()
          .catch(() => null)) as Partial<TakeoverRequest> | null
        if (
          !body ||
          typeof body.task !== 'string' ||
          body.task === '' ||
          (body.viewer !== undefined && !isScreenSize(body.viewer))
        )
          return failure(
            400,
            'invalid_request',
            'expected {"task": "<neoTaskId>", "viewer"?: {"width", "height"}}',
          )
        if (stopping) return failure(503, 'stopping', 'The viewer is stopping.')
        const start = takeOver(body.task, body.viewer)
        starting.add(start)
        try {
          return await start
        } finally {
          starting.delete(start)
        }
      }),
    },

    '/api/live/takeovers/:takeoverId/release': {
      POST: guarded(
        { sameOrigin: true },
        async (req: RouteRequest<'takeoverId'>) => {
          const id = req.params.takeoverId
          const released = await release(host, id)
          // Neo's own refusal (no such takeover) means there is nothing left to hold.
          if ('error' in released) {
            if (released.state.state === 'refused') held.delete(id)
            return released.error
          }
          held.delete(id)
          return Response.json({
            takeover: released.value,
          } satisfies TakeoverResponse)
        },
      ),
    },
  }

  async function takeOver(
    task: string,
    viewer: ScreenSize | undefined,
  ): Promise<Response> {
    const tabs = await readTabs(host)
    if ('error' in tabs) return tabs.error
    const tab = await currentTab(host, task, tabs.items)
    if (!tab)
      return failure(
        409,
        'no_tab',
        'This task has no open tab yet: nothing to take over.',
      )
    const started = await neoJson<LiveTakeover>(host, '/v1/takeovers', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(
        viewer ? { tabId: tab.tabId, viewer } : { tabId: tab.tabId },
      ),
    })
    if ('error' in started) return started.error
    if (stopping) {
      // `stop` has begun: give this one back now rather than leave the agent locked out.
      await release(host, started.value.id)
      return failure(503, 'stopping', 'The viewer is stopping.')
    }
    held.set(started.value.id, started.value)
    return Response.json({ takeover: started.value } satisfies TakeoverResponse)
  }

  return {
    routes,
    /** Gives back every takeover this viewer still holds, so the agent is not left locked out. */
    async stop() {
      stopping = true
      await Promise.race([
        Promise.allSettled([...starting]),
        Bun.sleep(RELEASE_ON_STOP_MS),
      ])
      const ids = [...held.keys()]
      held.clear()
      await Promise.all(
        ids.map((id) =>
          Promise.race([release(host, id), Bun.sleep(RELEASE_ON_STOP_MS)]),
        ),
      )
    },
  }
}

async function status(
  host: LiveHost,
  held: Map<string, LiveTakeover>,
  task: string | null,
): Promise<LiveStatus> {
  const tabs = await readTabs(host)
  if ('error' in tabs) return { host: tabs.state, tab: null, takeover: null }
  const tab = task ? await currentTab(host, task, tabs.items) : null
  // The takeover on the task's tab first; otherwise any the viewer still holds, so a reloaded
  // page can give back one whose agent has since worked in another tab.
  const candidates = [...held.values()].sort(
    (a, b) => Number(b.tabId === tab?.tabId) - Number(a.tabId === tab?.tabId),
  )
  for (const known of candidates) {
    // Neo releases a takeover on its own (TTL, tab closed): ask before reporting it held.
    const fresh = await neoJson<LiveTakeover>(
      host,
      `/v1/takeovers/${encodeURIComponent(known.id)}`,
    )
    if ('error' in fresh) {
      // Forget it only when Neo answered; an unreachable Neo may still hold it.
      if (fresh.state.state === 'refused') held.delete(known.id)
      continue
    }
    if (fresh.value.state !== 'active') {
      held.delete(known.id)
      continue
    }
    held.set(known.id, fresh.value)
    return { host: { state: 'up' }, tab, takeover: fresh.value }
  }
  return { host: { state: 'up' }, tab, takeover: null }
}

/**
 * The tab the task is on, the one Neo's `followTask` keeps in front. Null when that tab has
 * closed or the task has used none.
 */
async function currentTab(
  host: LiveHost,
  task: string,
  open: LiveTab[],
): Promise<LiveTab | null> {
  const trace = await neoJson<{ operations: TraceOperation[] }>(
    host,
    `/v1/trace?taskId=${encodeURIComponent(task)}&tail=${TRACE_TAIL}`,
  )
  if ('error' in trace) return null
  const tabId = taskLastTab(trace.value.operations)
  return open.find((tab) => tab.tabId === tabId) ?? null
}

/** The fields of Neo's `Operation` that place a task on a tab. */
export interface TraceOperation {
  /** `<runId>.<seq>`; run ids are UUIDv7, so they sort by creation. */
  id: string
  name: string
  /** The tab it targeted, when it had one. */
  tabId?: number
  /** The page after it ended. */
  page?: { tabId?: number }
  startedAt: number
}

/**
 * Neo's own rule for a task's current tab (`TraceStore::task_last_act`), over its trace: the
 * latest operation by start, skipping subscriptions, counts with its target tab; one without a
 * target counts with the tab of the page it ended on, which is how `context.newPage` makes its
 * new tab the task's.
 */
export function taskLastTab(operations: TraceOperation[]): number | null {
  let latest: { op: TraceOperation; tab: number } | null = null
  for (const op of operations) {
    if (SUBSCRIPTIONS.has(op.name)) continue
    const tab = op.tabId ?? op.page?.tabId
    if (tab === undefined) continue
    if (!latest || startsAfter(op, latest.op)) latest = { op, tab }
  }
  return latest?.tab ?? null
}

/** Neo's order: start time, then run id, then sequence within the run. */
function startsAfter(a: TraceOperation, b: TraceOperation): boolean {
  if (a.startedAt !== b.startedAt) return a.startedAt > b.startedAt
  const [runA = '', seqA = '0'] = a.id.split('.')
  const [runB = '', seqB = '0'] = b.id.split('.')
  if (runA !== runB) return runA > runB
  return Number(seqA) > Number(seqB)
}

async function readTabs(
  host: LiveHost,
): Promise<{ items: LiveTab[] } | { error: Response; state: HostState }> {
  const tabs = await neoJson<{ items: LiveTab[] }>(host, '/v1/tabs')
  if (!('error' in tabs))
    return {
      items: tabs.value.items.map(({ tabId, url, title }) => ({
        tabId,
        url,
        title,
      })),
    }
  return { error: tabs.error, state: tabs.state }
}

async function release(host: LiveHost, id: string) {
  return neoJson<LiveTakeover>(
    host,
    `/v1/takeovers/${encodeURIComponent(id)}/release`,
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ reason: 'done' }),
    },
  )
}

/**
 * A host API call answered as JSON, or the page's error response and what it says about the
 * host. Neo's own error keeps its status, code and message; a Neo that cannot be reached is a
 * 502 `host_unreachable`.
 */
async function neoJson<T>(
  host: LiveHost,
  path: string,
  init?: RequestInit,
): Promise<{ value: T } | { error: Response; state: HostState }> {
  let response: Response
  try {
    response = await host.fetch(path, init)
  } catch (cause) {
    const message = `Neo's host API is not reachable: ${cause instanceof Error ? cause.message : String(cause)}`
    return {
      error: failure(502, 'host_unreachable', message),
      state: { state: 'unreachable', message },
    }
  }
  const body = (await response.json().catch(() => null)) as
    | (T & { code?: unknown; message?: unknown })
    | null
  if (response.ok && body !== null) return { value: body }
  const code = typeof body?.code === 'string' ? body.code : 'neo_error'
  const message =
    typeof body?.message === 'string'
      ? body.message
      : `Neo answered HTTP ${response.status}`
  return {
    error: failure(response.ok ? 502 : response.status, code, message),
    state: { state: 'refused', code, message },
  }
}

/** Neo's `ViewerSize`: whole pixels from 100 to 4096 each way. */
function isScreenSize(value: unknown): value is ScreenSize {
  const size = value as Partial<ScreenSize> | null
  return [size?.width, size?.height].every(
    (side) =>
      typeof side === 'number' &&
      Number.isInteger(side) &&
      side >= 100 &&
      side <= 4096,
  )
}

function failure(status: number, code: string, message: string): Response {
  return Response.json({ error: { code, message } } satisfies LiveError, {
    status,
  })
}

/** `Host` must name this machine; with `sameOrigin`, `Origin` must be the viewer's own page. */
function guarded<R extends Request>(
  { sameOrigin }: { sameOrigin: boolean },
  handler: Handler<R>,
): Handler<R> {
  return (req, server) => {
    if (!isLocalRequest(req))
      return new Response('forbidden host\n', { status: 403 })
    if (
      sameOrigin &&
      req.headers.get('origin') !== `http://${req.headers.get('host')}`
    )
      return new Response('forbidden origin\n', { status: 403 })
    return handler(req, server)
  }
}
