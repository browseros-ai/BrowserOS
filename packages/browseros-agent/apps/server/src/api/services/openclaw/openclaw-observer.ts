/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Connects to the OpenClaw gateway's WebSocket control plane and pipes
 * chat broadcast events into a ClawSession state machine. The observer
 * is a transport layer only — it handles the WS connection lifecycle
 * (connect, handshake, reconnect) and delegates all state management
 * to ClawSession.
 */

import WebSocket from 'ws'
import { logger } from '../../../lib/logger'
import type { ClawSession } from './claw-session'

// ---------------------------------------------------------------------------
// Protocol types (subset of OpenClaw gateway protocol v3)
// ---------------------------------------------------------------------------

const PROTOCOL_VERSION = 3
const HANDSHAKE_REQUEST_ID = 'connect'
const RECONNECT_DELAY_MS = 5_000
const CONNECT_TIMEOUT_MS = 10_000

interface RequestFrame {
  type: 'req'
  id: string
  method: string
  params: Record<string, unknown>
}

type IncomingFrame =
  | { type: 'res'; id: string; ok: true; payload?: unknown }
  | {
      type: 'res'
      id: string
      ok: false
      error: { code: string; message: string }
    }
  | { type: 'event'; event: string; payload?: unknown }

// ---------------------------------------------------------------------------
// Observer
// ---------------------------------------------------------------------------

export class OpenClawObserver {
  private ws: WebSocket | null = null
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null
  private connected = false
  private closed = false
  private gatewayUrl: string | null = null
  private gatewayToken: string | null = null

  constructor(private readonly session: ClawSession) {}

  /** Start observing the gateway at the given URL with the given token. */
  connect(gatewayUrl: string, token: string): void {
    this.gatewayUrl = gatewayUrl
    this.gatewayToken = token
    this.closed = false
    this.doConnect()
  }

  /** Stop observing and close the WebSocket. */
  disconnect(): void {
    this.closed = true
    this.clearReconnect()
    if (this.ws) {
      try {
        this.ws.close()
      } catch {}
      this.ws = null
    }
    this.connected = false
  }

  /** Whether the observer has an active WS connection. */
  isConnected(): boolean {
    return this.connected
  }

  // ── Private ─────────────────────────────────────────────────────────

  private doConnect(): void {
    if (this.closed || !this.gatewayUrl || !this.gatewayToken) return

    const wsUrl = this.gatewayUrl
      .replace(/^http:\/\//, 'ws://')
      .replace(/^https:\/\//, 'wss://')

    logger.debug('OpenClaw observer connecting', { url: wsUrl })

    const ws = new WebSocket(wsUrl)
    this.ws = ws

    const connectTimeout = setTimeout(() => {
      logger.warn('OpenClaw observer handshake timeout')
      ws.terminate()
    }, CONNECT_TIMEOUT_MS)

    let handshakeSent = false

    ws.on('message', (raw) => {
      let frame: IncomingFrame
      try {
        frame = JSON.parse(raw.toString('utf8')) as IncomingFrame
      } catch {
        return
      }

      // The gateway sends a connect.challenge event before accepting
      // the connect request. Send the handshake after receiving it.
      if (
        frame.type === 'event' &&
        frame.event === 'connect.challenge' &&
        !handshakeSent
      ) {
        handshakeSent = true
        const connectReq: RequestFrame = {
          type: 'req',
          id: HANDSHAKE_REQUEST_ID,
          method: 'connect',
          params: {
            minProtocol: PROTOCOL_VERSION,
            maxProtocol: PROTOCOL_VERSION,
            client: {
              id: 'openclaw-tui',
              displayName: 'browseros-observer',
              version: '1.0.0',
              platform: 'node',
              mode: 'ui',
            },
            role: 'operator',
            scopes: ['operator.read'],
            auth: { token: this.gatewayToken },
          },
        }
        ws.send(JSON.stringify(connectReq))
        return
      }

      // Handshake response
      if (frame.type === 'res' && frame.id === HANDSHAKE_REQUEST_ID) {
        clearTimeout(connectTimeout)
        if (frame.ok) {
          this.connected = true
          logger.info('OpenClaw observer connected')
        } else {
          logger.warn('OpenClaw observer handshake failed', {
            error: frame.error,
          })
          ws.close()
        }
        return
      }

      // Broadcast events (only process after handshake completes)
      if (frame.type === 'event' && this.connected) {
        this.handleEvent(frame.event, frame.payload)
      }
    })

    ws.on('close', () => {
      clearTimeout(connectTimeout)
      this.connected = false
      this.ws = null

      // Reset any agents stuck in "working" to "unknown" — we missed
      // the final/end event because the WS closed mid-task. The
      // ClawSession will re-infer correct state from JSONL when the
      // observer reconnects and ensureObserverConnected() re-seeds.
      for (const [agentId, state] of this.session.getAllStates()) {
        if (state.status === 'working') {
          this.session.transition(agentId, 'unknown')
        }
      }

      if (!this.closed) {
        logger.debug('OpenClaw observer disconnected, scheduling reconnect')
        this.scheduleReconnect()
      }
    })

    ws.on('error', (err) => {
      clearTimeout(connectTimeout)
      logger.debug('OpenClaw observer WS error', {
        message: err.message,
      })
    })
  }

  private handleEvent(eventName: string, payload: unknown): void {
    if (eventName === 'chat') {
      this.handleChatEvent(payload)
    }
  }

  /**
   * Parse a gateway chat broadcast event and transition the ClawSession
   * state machine accordingly.
   */
  private handleChatEvent(payload: unknown): void {
    if (!payload || typeof payload !== 'object') return
    const p = payload as Record<string, unknown>

    const sessionKey = typeof p.sessionKey === 'string' ? p.sessionKey : null
    const state = typeof p.state === 'string' ? p.state : null

    if (!sessionKey || !state) return

    const agentId = extractAgentId(sessionKey)
    if (!agentId) return

    if (state === 'delta' || state === 'streaming') {
      this.session.transition(agentId, 'working', {
        sessionKey,
        currentTool: extractToolName(p),
      })
    } else if (state === 'final' || state === 'end') {
      this.session.transition(agentId, 'idle', { sessionKey })
    } else if (state === 'error') {
      const errorMsg =
        typeof p.errorMessage === 'string'
          ? p.errorMessage
          : typeof p.error === 'string'
            ? p.error
            : 'Unknown error'
      this.session.transition(agentId, 'error', { sessionKey, error: errorMsg })
    }
  }

  private scheduleReconnect(): void {
    this.clearReconnect()
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      this.doConnect()
    }, RECONNECT_DELAY_MS)
  }

  private clearReconnect(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer)
      this.reconnectTimer = null
    }
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Extract agentId from an OpenClaw session key.
 * Format: "agent:<agentId>:..." — we take the segment after "agent:".
 */
function extractAgentId(sessionKey: string): string | null {
  if (!sessionKey.startsWith('agent:')) return null
  const colonIdx = sessionKey.indexOf(':', 6)
  if (colonIdx === -1) return sessionKey.slice(6)
  return sessionKey.slice(6, colonIdx)
}

/**
 * Try to extract a tool name from a chat event payload.
 */
function extractToolName(payload: Record<string, unknown>): string | null {
  if (typeof payload.toolName === 'string') return payload.toolName
  if (typeof payload.tool === 'string') return payload.tool
  const content = payload.content
  if (content && typeof content === 'object' && 'name' in content) {
    const name = (content as Record<string, unknown>).name
    if (typeof name === 'string') return name
  }
  return null
}
