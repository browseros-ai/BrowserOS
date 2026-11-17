/**
 * The live screen's routes and shapes: what the viewer server answers and the live-screen panel
 * reads. The server owns them; the panel imports them and never redefines one.
 *
 * The browser cannot reach Neo's host API (it needs a bearer token, and a browser cannot set
 * `Authorization` on a WebSocket), so the viewer server stands in for it:
 *
 * ```text
 *  page (noVNC) ──ws /api/live/display?mode=…──▶ viewer ──ws /v1/display?mode=… + token──▶ Neo
 *  page ──/api/live/status, /api/live/takeovers──▶ viewer ──/v1/tabs, /v1/trace, /v1/takeovers──▶ Neo
 * ```
 *
 * Routes:
 *   GET  /api/live/display?<DisplayRequest>      WebSocket: RFB 3.8 both ways, relayed to Neo
 *   GET  /api/live/status                        LiveStatus
 *   POST /api/live/takeovers {tabId, viewer?}    TakeoverResponse | LiveError
 *   POST /api/live/takeovers/:id/release         TakeoverResponse | LiveError
 */

export const liveRoutes = {
  display: '/api/live/display',
  status: '/api/live/status',
  takeovers: '/api/live/takeovers',
  release: (takeoverId: string) =>
    `/api/live/takeovers/${encodeURIComponent(takeoverId)}/release`,
} as const

/** Neo's limit on a host task id (`followTask`, `_meta["com.browseros.neo/run"].taskId`). */
const MAX_TASK_ID_BYTES = 128

/**
 * What a view display keeps in front ("Follow agent"), as Neo's display takes it:
 * - `taskId`: the task's current tab, wherever the task goes next (`followTask`);
 * - `tabId`: that one tab (`followTab`).
 */
export type Follow = { taskId: string } | { tabId: number }

/**
 * Which display to open, as Neo's `GET /v1/display` takes it:
 * - `view`: watch the screen; with `follow`, Neo keeps that tab or task in front.
 * - `control`: click and type. It follows nothing, so the front tab stays where it is.
 * - `control` with `release`: the owner's display of a takeover; releasing the takeover closes it.
 */
export type DisplayRequest =
  | { mode: 'view'; follow?: Follow }
  | { mode: 'control'; release?: string }

export function displayQuery(request: DisplayRequest): string {
  const params = new URLSearchParams({ mode: request.mode })
  if (request.mode === 'view' && request.follow) {
    if ('taskId' in request.follow)
      params.set('followTask', request.follow.taskId)
    else params.set('followTab', String(request.follow.tabId))
  }
  if (request.mode === 'control' && request.release)
    params.set('release', request.release)
  return params.toString()
}

/**
 * Reads a display request back, refusing what Neo would refuse (a contradictory or unknown
 * parameter), so the relay only ever forwards a query it built itself.
 */
export function parseDisplayQuery(
  params: URLSearchParams,
): DisplayRequest | { error: string } {
  const known = new Set(['mode', 'followTask', 'followTab', 'release'])
  for (const name of params.keys()) {
    if (!known.has(name)) return { error: `unknown parameter \`${name}\`` }
    if (params.getAll(name).length > 1)
      return { error: `\`${name}\` given twice` }
  }
  const followTask = params.get('followTask')
  const followTab = params.get('followTab')
  const release = params.get('release')
  switch (params.get('mode')) {
    case 'view':
      if (release !== null)
        return { error: 'a view display takes no `release`' }
      return parseFollow(followTask, followTab)
    case 'control':
      if (followTask !== null || followTab !== null)
        return { error: 'a control display follows nothing' }
      if (release === null) return { mode: 'control' }
      if (release === '') return { error: '`release` is empty' }
      return { mode: 'control', release }
    default:
      return { error: '`mode` must be view or control' }
  }
}

function parseFollow(
  followTask: string | null,
  followTab: string | null,
): DisplayRequest | { error: string } {
  if (followTask !== null && followTab !== null)
    return { error: '`followTab` and `followTask` cannot both be given' }
  if (followTab !== null) {
    const tabId = Number(followTab)
    if (!/^[0-9]+$/.test(followTab) || !Number.isSafeInteger(tabId))
      return { error: '`followTab` must be a tab id' }
    return { mode: 'view', follow: { tabId } }
  }
  if (followTask === null) return { mode: 'view' }
  if (followTask === '') return { error: '`followTask` is empty' }
  if (new TextEncoder().encode(followTask).length > MAX_TASK_ID_BYTES)
    return { error: `\`followTask\` is over ${MAX_TASK_ID_BYTES} bytes` }
  return { mode: 'view', follow: { taskId: followTask } }
}

/**
 * Why a display closed, as the page's WebSocket sees it. Neo's own codes pass through the relay
 * unchanged (`released`, and 1003, 1008, 1011 with Neo's reason); the 49xx codes are the
 * viewer's, for what happened between it and Neo. The close reason is a sentence for the owner.
 */
export const displayClose = {
  /** Neo's: the display's takeover was released (by the owner, its TTL, or its tab closing). */
  released: 4002,
  /** Neo's host API did not answer: no container, or one started without its host API. */
  hostUnreachable: 4901,
  /** Neo answered but did not open the display; the reason carries Neo's error code and message. */
  displayRefused: 4902,
  /** The open connection to Neo dropped without a close frame (the container stopped). */
  displayLost: 4903,
} as const

/** A tab the panel can name and take over (`GET /v1/tabs`). */
export interface LiveTab {
  tabId: number
  url: string
  title: string
}

/** Neo's `Takeover` (openapi `components.schemas.Takeover`), the fields the panel reads. */
export interface LiveTakeover {
  id: string
  tabId: number
  /** `active` or `released`; Neo may add values. */
  state: string
  /** Why it was released: `done`, `idle`, `host`, `expired`, `tab_closed`. */
  releaseReason?: string
  /** The control display to open for the owner, as a host API path. */
  display: string
  /** Epoch milliseconds. */
  createdAt: number
  /** When the takeover's lock ends; Neo releases the takeover 10 s before. */
  expiresAt: number
  releasedAt?: number
}

/** Whether the viewer can reach Neo's host API, and if not, a sentence saying why. */
export type HostState =
  | { state: 'up' }
  | { state: 'unreachable'; message: string }
  | { state: 'refused'; code: string; message: string }

/**
 * Where the agent is: the tab of the most recent run's last operation that acted on a tab, while
 * that tab is open. "Follow agent" keeps it in front, Take over takes it. `taskId` is the task
 * that run's request named (`_meta` taskId), when it named one: then Neo follows the task, so the
 * screen moves with it to a new tab without waiting for the next status. A task a script picks
 * itself (`neo.task`) is not in Neo's trace; such a run is followed by its tab.
 */
export interface AgentFocus {
  tab: LiveTab
  taskId: string | null
}

export interface LiveStatus {
  host: HostState
  /** Null before the agent has acted on a tab, or once that tab has closed. */
  agent: AgentFocus | null
  /** The takeover this viewer started, while active: on the agent's tab when it has one there. */
  takeover: LiveTakeover | null
}

/** Neo's screen in pixels, as its RFB server announced it (Neo's `ViewerSize`, 100 to 4096). */
export interface ScreenSize {
  width: number
  height: number
}

export interface TakeoverRequest {
  /** The tab to take: the agent's, as the panel shows it (`LiveStatus.agent.tab`). */
  tabId: number
  /**
   * Neo's own screen size, as the panel shows it. Sent on as the takeover's `viewer`, so Neo
   * fits the tab, and a fit brings the tab to the front: without one, a takeover locks the tab
   * where it is, maybe behind the tab the owner sees. The same size keeps the screen as it is
   * (a desktop fit: maximized window, no emulation); Neo puts the window back on release.
   */
  viewer?: ScreenSize
}

export interface TakeoverResponse {
  takeover: LiveTakeover
}

/** An error from a live-screen route; `code` is Neo's when Neo refused (`not_found`, …). */
export interface LiveError {
  error: { code: string; message: string }
}
