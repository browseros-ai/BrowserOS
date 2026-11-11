/**
 * One page's live screen: the page's WebSocket on the viewer, joined to Neo's
 * `GET /v1/display` on the host API, frame for frame in both directions.
 *
 * ```text
 *  page ◀──ws──▶ DisplayRelay ◀──ws + Authorization──▶ Neo /v1/display?mode=…
 * ```
 *
 * The relay adds only the bearer token, which stays on this side. It does not read RFB: every
 * binary frame passes as it came, and text frames too (Neo closes a display that gets one, with
 * 1003). Closes pass through with their code and reason, so the page sees Neo's `4002 released`
 * when a takeover ends. What Neo cannot say, the relay says with its own codes
 * (`displayClose` in `shared/live-screen.ts`): Neo unreachable, the display refused, the
 * connection lost.
 *
 * Lifecycle: the page's socket opens first, then the relay dials Neo. Whichever side closes,
 * the relay closes the other, and a page that goes away closes Neo's display with 1000.
 */

import { displayClose } from '../../shared/live-screen'
import type { LiveHost } from './live-host'
import type { SessionSocket, SocketSession } from './socket-session'

/** How long Neo has to accept the display before the page is told it is unreachable. */
const CONNECT_TIMEOUT_MS = 10_000
/** Page frames held while Neo's side opens. RFB has the server speak first, so this stays empty. */
const MAX_PENDING_FRAMES = 64
/** A WebSocket close reason is at most 123 bytes of UTF-8. */
const MAX_REASON_BYTES = 123

type Frame = string | ArrayBuffer

/**
 * Bun's WebSocket client takes request headers; a browser's cannot, which is why this relay
 * exists. The tool's tsconfig includes the DOM library for the page, and its `WebSocket` type
 * hides Bun's constructor, so the client is typed here.
 */
const ClientWebSocket = WebSocket as unknown as new (
  url: string,
  options: { headers: Record<string, string> },
) => WebSocket

export class DisplayRelay implements SocketSession {
  private page: SessionSocket | null = null
  private upstream: WebSocket | null = null
  private upstreamOpen = false
  private pending: Frame[] = []
  private ended = false
  private connectTimer: ReturnType<typeof setTimeout> | null = null

  /** `path` is the host API path to open, e.g. `/v1/display?mode=view&followTask=t1`. */
  constructor(
    private readonly host: LiveHost,
    private readonly path: string,
  ) {}

  open(page: SessionSocket): void {
    this.page = page
    let target: { url: string; headers: Record<string, string> }
    try {
      target = this.host.socket(this.path)
    } catch (cause) {
      this.closePage(
        displayClose.hostUnreachable,
        `Neo's host API is not set up: ${messageOf(cause)}`,
      )
      return
    }
    const upstream = new ClientWebSocket(target.url, {
      headers: target.headers,
    })
    upstream.binaryType = 'arraybuffer'
    this.upstream = upstream
    this.connectTimer = setTimeout(() => {
      if (this.upstreamOpen || this.ended) return
      upstream.close()
      this.closePage(
        displayClose.hostUnreachable,
        `Neo's display did not answer within ${CONNECT_TIMEOUT_MS / 1000} s`,
      )
    }, CONNECT_TIMEOUT_MS)

    upstream.addEventListener('open', () => {
      this.clearConnectTimer()
      if (this.ended) {
        upstream.close(1000)
        return
      }
      this.upstreamOpen = true
      for (const frame of this.pending.splice(0)) upstream.send(frame)
    })
    upstream.addEventListener('message', (event) => {
      this.toPage(event.data as string | ArrayBuffer)
    })
    upstream.addEventListener('close', (event) => {
      this.clearConnectTimer()
      if (this.ended) return
      if (this.upstreamOpen) {
        const [code, reason] = pageCloseFor(event.code, event.reason)
        this.closePage(code, reason)
      } else {
        void this.explainRefusal(event.code, event.reason)
      }
    })
  }

  message(_page: SessionSocket, message: string | Buffer): void {
    if (this.ended || !this.upstream) return
    // Page frames are input events, a few bytes each; copying one out of Bun's buffer is cheap.
    const frame: Frame =
      typeof message === 'string' ? message : new Uint8Array(message).buffer
    if (this.upstreamOpen) {
      this.upstream.send(frame)
      return
    }
    if (this.pending.length >= MAX_PENDING_FRAMES) {
      this.closeBoth(1008, 'too many messages before the display opened')
      return
    }
    this.pending.push(frame)
  }

  close(_page: SessionSocket, _code: number, _reason: string): void {
    this.page = null
    this.end()
    // The page left: Neo frees the display. A takeover outlives its display; only
    // `POST /v1/takeovers/{id}/release` gives the tab back.
    this.upstream?.close(1000)
  }

  private toPage(frame: Frame): void {
    if (this.ended || !this.page) return
    // 0 means Bun dropped the frame: the page fell more than the backpressure limit behind.
    // RFB is one byte stream, so a missing frame would corrupt everything after it.
    if (this.page.send(frame) === 0)
      this.closeBoth(1011, 'the page could not keep up with the screen')
  }

  /**
   * Bun's client reports a refused handshake only as 1002 "Expected 101 status code", and an
   * unreachable host as 1006. Neo checks the query and the takeover before the upgrade, so the
   * same path asked as a plain GET answers with Neo's own error (404 `not_found`, 409
   * `takeover_released`, 401 `unauthorized`). Admission runs after the upgrade check, so a plain
   * GET of a good query is a 400 for "not a WebSocket"; then the screen itself did not open
   * (`503 display_unavailable` while the browser or Xvnc starts).
   */
  private async explainRefusal(code: number, reason: string): Promise<void> {
    let response: Response
    try {
      response = await this.host.fetch(this.path)
    } catch (cause) {
      this.closePage(
        displayClose.hostUnreachable,
        `Neo's host API is not reachable: ${messageOf(cause)}`,
      )
      return
    }
    const error = await neoError(response)
    if (response.status === 400 || !error) {
      this.closePage(
        displayClose.displayRefused,
        `display_unavailable: Neo's screen did not open (${reason || code})`,
      )
      return
    }
    this.closePage(
      displayClose.displayRefused,
      `${error.code}: ${error.message}`,
    )
  }

  private closePage(code: number, reason: string): void {
    this.end()
    this.page?.close(code, truncate(reason))
    this.page = null
  }

  private closeBoth(code: number, reason: string): void {
    this.closePage(code, reason)
    this.upstream?.close(1000)
  }

  private end(): void {
    this.ended = true
    this.pending = []
    this.clearConnectTimer()
  }

  private clearConnectTimer(): void {
    if (this.connectTimer) clearTimeout(this.connectTimer)
    this.connectTimer = null
  }
}

/**
 * The code to give the page when Neo's side closed after opening. Neo's own codes pass as they
 * are; 1005 (no code) is a plain close, and 1006 or 1015 (no close frame: the container
 * stopped, the network broke) cannot be sent by a server, so they become `displayLost`.
 */
export function pageCloseFor(code: number, reason: string): [number, string] {
  if (code === 1005) return [1000, reason]
  if (code === 1006 || code === 1015)
    return [displayClose.displayLost, 'the connection to Neo dropped']
  const sendable =
    (code >= 1000 && code <= 1003) ||
    (code >= 1007 && code <= 1011) ||
    (code >= 3000 && code <= 4999)
  return sendable ? [code, reason] : [1011, reason]
}

async function neoError(
  response: Response,
): Promise<{ code: string; message: string } | null> {
  try {
    const body = (await response.json()) as {
      code?: unknown
      message?: unknown
    }
    return typeof body.code === 'string' && typeof body.message === 'string'
      ? { code: body.code, message: body.message }
      : null
  } catch {
    return null
  }
}

function messageOf(cause: unknown): string {
  return cause instanceof Error ? cause.message : String(cause)
}

function truncate(reason: string): string {
  const bytes = new TextEncoder().encode(reason)
  if (bytes.length <= MAX_REASON_BYTES) return reason
  let cut = MAX_REASON_BYTES - 3
  // Back off to a character boundary (a UTF-8 continuation byte is 10xxxxxx).
  while (cut > 0 && ((bytes[cut] ?? 0) & 0xc0) === 0x80) cut--
  return `${new TextDecoder().decode(bytes.slice(0, cut))}…`
}
