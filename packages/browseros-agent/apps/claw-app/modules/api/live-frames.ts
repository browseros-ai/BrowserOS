import { useEffect, useState } from 'react'
import { useApiBaseUrl } from './audit.hooks'

/**
 * Live frames of running sessions for the cockpit's cards, from claw-server's
 * `/api/v1/screencasts` WebSocket.
 *
 * One socket serves every card on the page: a long-lived stream per card would
 * hold one of Chrome's six connections to the server each, and enough running
 * sessions would stall every other API call the cockpit makes. Cards subscribe
 * to a watch (a session, optionally one of its tabs); the feed sends the server
 * the whole set of watches whenever it changes and after every reconnect, and
 * the server answers with frames only while the set names them.
 *
 * The socket is open only while some card subscribes and the page is visible,
 * so a hidden cockpit lets the server stop capturing.
 */

interface LiveWatch {
  sessionId: string
  /** Omitted follows the session's current tab. */
  browserTabId?: number
}

type FrameListener = (jpeg: Blob) => void

const RECONNECT_MIN_MS = 1000
const RECONNECT_MAX_MS = 15_000

function watchKey(watch: LiveWatch): string {
  return JSON.stringify([watch.sessionId, watch.browserTabId ?? null])
}

function socketUrl(baseUrl: string): string {
  return `${baseUrl.replace(/\/+$/, '').replace(/^http/, 'ws')}/api/v1/screencasts`
}

class LiveFrameFeed {
  private readonly watches = new Map<
    string,
    { watch: LiveWatch; listeners: Set<FrameListener> }
  >()
  private socket: WebSocket | null = null
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private reconnectDelay = RECONNECT_MIN_MS
  private updateQueued = false
  private followsVisibility = false
  /** The watch set the open socket last sent, so an unchanged set isn't resent. */
  private lastSent: string | null = null

  constructor(private readonly url: string) {}

  subscribe(watch: LiveWatch, listener: FrameListener): () => void {
    const key = watchKey(watch)
    let entry = this.watches.get(key)
    if (!entry) {
      entry = { watch, listeners: new Set() }
      this.watches.set(key, entry)
    }
    entry.listeners.add(listener)
    this.queueUpdate()
    return () => {
      const current = this.watches.get(key)
      if (!current?.listeners.delete(listener)) return
      if (current.listeners.size === 0) this.watches.delete(key)
      this.queueUpdate()
    }
  }

  /**
   * Applies the subscription changes of one render together: a card that
   * moves to another tab unsubscribes and resubscribes in one commit, which
   * must not close and reopen the socket.
   */
  private queueUpdate(): void {
    if (this.updateQueued) return
    this.updateQueued = true
    queueMicrotask(() => {
      this.updateQueued = false
      this.sync()
      this.sendWatches()
    })
  }

  /** Opens or closes the socket to match: open while watched and visible. */
  private readonly sync = (): void => {
    const watched = this.watches.size > 0
    if (watched !== this.followsVisibility) {
      this.followsVisibility = watched
      if (watched) document.addEventListener('visibilitychange', this.sync)
      else document.removeEventListener('visibilitychange', this.sync)
    }
    if (!watched || document.visibilityState !== 'visible') {
      this.stop()
      this.reconnectDelay = RECONNECT_MIN_MS
    } else if (!this.socket && this.reconnectTimer === null) {
      this.connect()
    }
  }

  private connect(): void {
    const socket = new WebSocket(this.url)
    socket.binaryType = 'arraybuffer'
    socket.onopen = () => {
      this.reconnectDelay = RECONNECT_MIN_MS
      this.lastSent = null
      this.sendWatches()
    }
    socket.onmessage = (event: MessageEvent) => this.deliver(event.data)
    socket.onclose = () => {
      if (this.socket !== socket) return
      this.socket = null
      this.scheduleReconnect()
    }
    this.socket = socket
  }

  private scheduleReconnect(): void {
    const delay = this.reconnectDelay
    this.reconnectDelay = Math.min(delay * 2, RECONNECT_MAX_MS)
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      this.sync()
    }, delay)
  }

  private stop(): void {
    if (this.reconnectTimer !== null) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
    const socket = this.socket
    if (!socket) return
    this.socket = null
    socket.onopen = null
    socket.onmessage = null
    socket.onclose = null
    socket.close()
  }

  private sendWatches(): void {
    const socket = this.socket
    if (!socket || socket.readyState !== WebSocket.OPEN) return
    const watch = [...this.watches.values()].map((entry) => entry.watch)
    const message = JSON.stringify({ watch })
    if (message === this.lastSent) return
    this.lastSent = message
    socket.send(message)
  }

  /** Frame layout: u32 big-endian header length, JSON header, JPEG bytes. */
  private deliver(data: unknown): void {
    if (!(data instanceof ArrayBuffer) || data.byteLength < 4) return
    const headerLength = new DataView(data).getUint32(0)
    if (4 + headerLength > data.byteLength) return
    let header: LiveWatch
    try {
      header = JSON.parse(
        new TextDecoder().decode(new Uint8Array(data, 4, headerLength)),
      )
    } catch {
      return
    }
    const entry = this.watches.get(watchKey(header))
    if (!entry) return
    const jpeg = new Blob([new Uint8Array(data, 4 + headerLength)], {
      type: 'image/jpeg',
    })
    for (const listener of entry.listeners) listener(jpeg)
  }
}

const feeds = new Map<string, LiveFrameFeed>()

function feedFor(baseUrl: string): LiveFrameFeed {
  const url = socketUrl(baseUrl)
  let feed = feeds.get(url)
  if (!feed) {
    feed = new LiveFrameFeed(url)
    feeds.set(url, feed)
  }
  return feed
}

/**
 * The newest live frame of a session's current tab, or of `browserTabId`, as
 * an object URL; null until the first frame arrives. The URL is revoked when a
 * newer frame replaces it or the watch changes.
 */
export function useLiveFrame(
  sessionId: string,
  browserTabId?: number,
): string | null {
  const baseUrl = useApiBaseUrl()
  const [src, setSrc] = useState<string | null>(null)

  useEffect(() => {
    if (baseUrl === null) return
    let current: string | null = null
    const unsubscribe = feedFor(baseUrl).subscribe(
      { sessionId, browserTabId },
      (jpeg) => {
        const previous = current
        current = URL.createObjectURL(jpeg)
        setSrc(current)
        if (previous) URL.revokeObjectURL(previous)
      },
    )
    return () => {
      unsubscribe()
      if (current) URL.revokeObjectURL(current)
      setSrc(null)
    }
  }, [baseUrl, sessionId, browserTabId])

  return src
}
