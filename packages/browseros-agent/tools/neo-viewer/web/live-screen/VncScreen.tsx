/**
 * Neo's screen in the page: noVNC on a WebSocket to the viewer's relay
 * (`/api/live/display`). One connection per display request and attempt; a new request or a
 * retry closes the old one first.
 *
 * The panel opens the WebSocket itself and hands it to noVNC, because noVNC's own `disconnect`
 * event says only whether a close was clean, and the panel needs the close code: Neo's
 * `4002 released` and the relay's 49xx codes are how it learns a takeover ended or why the
 * screen is down.
 *
 * Opening is deferred a moment (`OPEN_DELAY_MS`). Neo's VNC server (TigerVNC's Xvnc) black-marks
 * a client address for every connection that ends before its handshake finishes, and blocks it
 * after five, for 10 s and then twice as long each time. Every display reaches Xvnc from Neo's
 * own 127.0.0.1, so a page that opens and drops connections (React's development double mount,
 * quick Follow/Control toggles) would block every display, the agent host's included.
 *
 * noVNC is loaded when the first screen opens: it reads `window` as it loads, so importing it
 * up front would keep the panel from rendering anywhere without a browser (its tests).
 */

import type RFB from '@novnc/novnc'
import { useEffect, useRef } from 'react'
import {
  type DisplayRequest,
  displayQuery,
  liveRoutes,
  type ScreenSize,
} from '../../shared/live-screen'

/** Long enough to outlast a mount that is undone at once, short enough not to be noticed. */
const OPEN_DELAY_MS = 150

export function VncScreen({
  display,
  attempt,
  onConnected,
  onClosed,
}: {
  display: DisplayRequest
  attempt: number
  onConnected: (screen: ScreenSize | null) => void
  onClosed: (code: number, reason: string) => void
}) {
  const target = useRef<HTMLDivElement>(null)
  // The socket outlives renders; it calls whatever the panel passed last.
  const report = useRef({ onConnected, onClosed })
  report.current = { onConnected, onClosed }
  const query = displayQuery(display)
  const viewOnly = display.mode === 'view'

  // biome-ignore lint/correctness/useExhaustiveDependencies: a new attempt is a new connection.
  useEffect(() => {
    const element = target.current
    if (!element) return
    // Cleared before this effect closes the socket, so a close the panel caused is never
    // reported as the screen going down.
    let live = true
    let rfb: RFB | null = null
    const opening = setTimeout(async () => {
      const { default: Rfb } = await import('@novnc/novnc')
      if (!live) return
      const scheme = location.protocol === 'https:' ? 'wss' : 'ws'
      const socket = new WebSocket(
        `${scheme}://${location.host}${liveRoutes.display}?${query}`,
      )
      socket.addEventListener('close', (event) => {
        if (live) report.current.onClosed(event.code, event.reason)
        live = false
      })
      rfb = new Rfb(element, socket, { shared: true })
      // View displays drop input on Neo's side too; viewOnly also keeps keys in the page.
      rfb.viewOnly = viewOnly
      rfb.focusOnClick = !viewOnly
      rfb.scaleViewport = true
      // Never resize Neo's screen to the panel: the agent's pages would change layout.
      rfb.resizeSession = false
      rfb.background = '#F8F9FB'
      rfb.addEventListener('connect', () => {
        // noVNC sizes its canvas to the framebuffer from the server's init, before `connect`.
        const canvas = element.querySelector('canvas')
        const screen =
          canvas && canvas.width > 0 && canvas.height > 0
            ? { width: canvas.width, height: canvas.height }
            : null
        if (live) report.current.onConnected(screen)
      })
    }, OPEN_DELAY_MS)
    return () => {
      live = false
      clearTimeout(opening)
      rfb?.disconnect()
    }
  }, [query, attempt, viewOnly])

  return (
    <div
      ref={target}
      className="h-full w-full bg-[#F8F9FB]"
      data-display={query}
    />
  )
}
