/**
 * The live screen's routes on the viewer (`shared/live-screen.ts` names them): the display
 * relay, the panel's status, and Take over / Give back over Neo's takeover flow.
 *
 * Who may call them: like the rest of `/api`, the request must name this machine in `Host`. The
 * display and the two POSTs also require an `Origin` of the viewer's own page. A WebSocket is
 * not covered by the same-origin policy and a form POST needs no preflight, so without that
 * check any web page the owner has open could drive the browser through `ws://127.0.0.1:7310`.
 *
 * Status: where the agent is (the tab of the most recent run's last operation), so "Follow
 * agent" works whether or not the agent names a task.
 *
 * Takeovers: `POST /v1/takeovers` locks the agent out of one tab until the takeover is released
 * or its TTL (20 min) runs out. The viewer remembers the takeovers it started, so a reloaded
 * page finds the one it holds, and gives them all back when the viewer stops, including one
 * whose start is still on its way to Neo.
 */

import type { Server } from 'bun'
import {
  type AgentFocus,
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
import { type LiveHost, NO_HOST } from './live-host'
import type { SocketSession } from './socket-session'

/** Neo's latest operations read back to find the tab the agent is on. */
const TRACE_TAIL = 100
/**
 * Operations that only subscribe, so they put the agent on no tab: Neo's own list
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

    [liveRoutes.status]: guarded({ sameOrigin: false }, async () =>
      Response.json((await status(host, held)) satisfies LiveStatus),
    ),

    [liveRoutes.takeovers]: {
      POST: guarded({ sameOrigin: true }, async (req) => {
        const body = (await req
          .json()
          .catch(() => null)) as Partial<TakeoverRequest> | null
        if (
          !body ||
          !Number.isSafeInteger(body.tabId) ||
          (body.tabId ?? -1) < 0 ||
          (body.viewer !== undefined && !isScreenSize(body.viewer))
        )
          return failure(
            400,
            'invalid_request',
            'expected {"tabId": <tab id>, "viewer"?: {"width", "height"}}',
          )
        if (stopping) return failure(503, 'stopping', 'The viewer is stopping.')
        const start = takeOver(body.tabId as number, body.viewer)
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

  /** Neo answers a tab that is no longer open with `404 tab_not_found`, passed on as it is. */
  async function takeOver(
    tabId: number,
    viewer: ScreenSize | undefined,
  ): Promise<Response> {
    const started = await neoJson<LiveTakeover>(host, '/v1/takeovers', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(viewer ? { tabId, viewer } : { tabId }),
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
): Promise<LiveStatus> {
  const tabs = await readTabs(host)
  if ('error' in tabs) return { host: tabs.state, agent: null, takeover: null }
  const agent = await agentOn(host, tabs.items)
  const tab = agent?.tab
  // The takeover on the agent's tab first; otherwise any the viewer still holds, so a reloaded
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
    return { host: { state: 'up' }, agent, takeover: fresh.value }
  }
  return { host: { state: 'up' }, agent, takeover: null }
}

/** The agent's tab, read from Neo's latest operations; null when it has acted on no open tab. */
async function agentOn(
  host: LiveHost,
  open: LiveTab[],
): Promise<AgentFocus | null> {
  const trace = await neoJson<TraceTail>(host, `/v1/trace?tail=${TRACE_TAIL}`)
  if ('error' in trace) return null
  const focus = agentFocus(trace.value)
  const tab = open.find((candidate) => candidate.tabId === focus?.tabId)
  return focus && tab ? { tab, taskId: focus.taskId } : null
}

/** The fields of Neo's `TraceTail` (`GET /v1/trace`) that place the agent on a tab. */
export interface TraceTail {
  operations: Array<{
    /** `<runId>.<seq>`, `seq` counting the run's operations in start order. */
    id: string
    runId: string
    name: string
    /** The tab it targeted, when it had one. */
    tabId?: number
    /** The page after it ended. */
    page?: { tabId?: number }
  }>
  runs: Array<{ id: string; taskId?: string; startedAt: number }>
}

/**
 * The agent's tab: the most recent run's last operation that acted on a tab. Subscriptions
 * (`events.*`) act on none; an operation without a target counts with the tab of the page it
 * ended on, which is how `context.newPage` puts the agent on its new tab (Neo's own rule for a
 * task's tab, `TraceStore::task_last_act`). A run that touched no tab (`return 1`) leaves the
 * agent where the run before it did. `taskId` is that run's own, when the agent sent one.
 */
export function agentFocus(
  trace: TraceTail,
): { tabId: number; taskId: string | null } | null {
  const runs = new Map(trace.runs.map((run) => [run.id, run]))
  let latest: { order: [number, string, number]; tabId: number } | null = null
  for (const op of trace.operations) {
    if (SUBSCRIPTIONS.has(op.name)) continue
    const tabId = op.tabId ?? op.page?.tabId
    if (tabId === undefined) continue
    // Runs by start (ids are UUIDv7, so the id breaks a tie by creation), then by sequence.
    const order: [number, string, number] = [
      runs.get(op.runId)?.startedAt ?? 0,
      op.runId,
      Number(op.id.slice(op.id.lastIndexOf('.') + 1)),
    ]
    if (!latest || after(order, latest.order)) latest = { order, tabId }
  }
  if (!latest) return null
  return {
    tabId: latest.tabId,
    taskId: runs.get(latest.order[1])?.taskId ?? null,
  }
}

function after(
  [startA, runA, seqA]: [number, string, number],
  [startB, runB, seqB]: [number, string, number],
): boolean {
  if (startA !== startB) return startA > startB
  if (runA !== runB) return runA > runB
  return seqA > seqB
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
    state: NO_HOST.has(code)
      ? { state: 'unreachable', message }
      : { state: 'refused', code, message },
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
