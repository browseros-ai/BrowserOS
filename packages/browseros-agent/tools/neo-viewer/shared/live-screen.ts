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
 *   GET  /api/live/status?task=<neoTaskId>       LiveStatus
 *   POST /api/live/takeovers {task}              TakeoverResponse | LiveError
 *   POST /api/live/takeovers/:id/release         TakeoverResponse | LiveError
 *
 * `task` is always Neo's task id (`TaskSummary.neoTaskId`), the one Neo's runs carry.
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
 * Which display to open, as Neo's `GET /v1/display` takes it:
 * - `view` with `followTask`: watch the screen and keep the task's current tab in front
 *   ("Follow agent"); without it, just watch.
 * - `control`: click and type. It takes no `followTask`, so the front tab stops following.
 * - `control` with `release`: the owner's display of a takeover; releasing the takeover closes it.
 */
export type DisplayRequest =
  | { mode: 'view'; followTask?: string }
  | { mode: 'control'; release?: string }

export function displayQuery(request: DisplayRequest): string {
  const params = new URLSearchParams({ mode: request.mode })
  if (request.mode === 'view' && request.followTask)
    params.set('followTask', request.followTask)
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
  const known = new Set(['mode', 'followTask', 'release'])
  for (const name of params.keys()) {
    if (!known.has(name)) return { error: `unknown parameter \`${name}\`` }
    if (params.getAll(name).length > 1)
      return { error: `\`${name}\` given twice` }
  }
  const mode = params.get('mode')
  const followTask = params.get('followTask')
  const release = params.get('release')
  if (mode === 'view') {
    if (release !== null) return { error: 'a view display takes no `release`' }
    if (followTask === null) return { mode }
    if (followTask === '') return { error: '`followTask` is empty' }
    if (new TextEncoder().encode(followTask).length > MAX_TASK_ID_BYTES)
      return { error: `\`followTask\` is over ${MAX_TASK_ID_BYTES} bytes` }
    return { mode, followTask }
  }
  if (mode === 'control') {
    if (followTask !== null)
      return { error: 'a control display takes no `followTask`' }
    if (release === null) return { mode }
    if (release === '') return { error: '`release` is empty' }
    return { mode, release }
  }
  return { error: '`mode` must be view or control' }
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

export interface LiveStatus {
  host: HostState
  /**
   * The tab the task's latest operation targeted, while it is open: the tab "Follow agent"
   * keeps in front and Take over takes. Null without a task, or before its first tab operation.
   */
  tab: LiveTab | null
  /** The takeover this viewer started on that tab, while active. */
  takeover: LiveTakeover | null
}

export interface TakeoverRequest {
  task: string
}

export interface TakeoverResponse {
  takeover: LiveTakeover
}

/** An error from a live-screen route; `code` is Neo's when Neo refused (`not_found`, …). */
export interface LiveError {
  error: { code: string; message: string }
}
