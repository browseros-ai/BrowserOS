/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Connects to the OpenClaw gateway's WebSocket control plane and tracks
 * per-agent session status in real time. The observer subscribes to `chat`
 * broadcast events — when an agent starts streaming, finishes a turn, or
 * hits an error, the status map updates immediately.
 *
 * This is a read-only listener. It does not issue any RPC commands.
 */

import WebSocket from 'ws'
import { logger } from '../../../lib/logger'

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
// Public types
// ---------------------------------------------------------------------------

export type AgentLiveStatus = 'working' | 'idle' | 'error' | 'unknown'

export interface AgentStatusEntry {
  status: AgentLiveStatus
  sessionKey: string | null
  lastEventAt: number
  currentTool: string | null
  error: string | null
}

export type StatusChangeListener = (
  agentId: string,
  entry: AgentStatusEntry,
) => void

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

  private readonly agentStatuses = new Map<string, AgentStatusEntry>()
  private readonly listeners = new Set<StatusChangeListener>()

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

  /** Get the live status of a specific agent. */
  getStatus(agentId: string): AgentStatusEntry {
    return (
      this.agentStatuses.get(agentId) ?? {
        status: 'unknown',
        sessionKey: null,
        lastEventAt: 0,
        currentTool: null,
        error: null,
      }
    )
  }

  /** Get live statuses for all tracked agents. */
  getAllStatuses(): Map<string, AgentStatusEntry> {
    return this.agentStatuses
  }

  /** Whether the observer has an active WS connection. */
  isConnected(): boolean {
    return this.connected
  }

  /** Subscribe to status changes. Returns unsubscribe function. */
  onStatusChange(listener: StatusChangeListener): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
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

    ws.on('open', () => {
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
    })

    ws.on('message', (raw) => {
      let frame: IncomingFrame
      try {
        frame = JSON.parse(raw.toString('utf8')) as IncomingFrame
      } catch {
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

      // Broadcast events
      if (frame.type === 'event') {
        this.handleEvent(frame.event, frame.payload)
      }
    })

    ws.on('close', () => {
      clearTimeout(connectTimeout)
      this.connected = false
      this.ws = null
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
    if (eventName === 'connect.challenge') return

    if (eventName === 'chat') {
      this.handleChatEvent(payload)
    }
  }

  private handleChatEvent(payload: unknown): void {
    if (!payload || typeof payload !== 'object') return
    const p = payload as Record<string, unknown>

    const sessionKey = typeof p.sessionKey === 'string' ? p.sessionKey : null
    const state = typeof p.state === 'string' ? p.state : null

    if (!sessionKey || !state) return

    // Extract agentId from session key: "agent:<agentId>:..."
    const agentId = extractAgentId(sessionKey)
    if (!agentId) return

    const now = Date.now()
    const prev = this.agentStatuses.get(agentId)

    if (state === 'delta' || state === 'streaming') {
      // Agent is actively streaming a response
      const toolName = extractToolName(p)
      const entry: AgentStatusEntry = {
        status: 'working',
        sessionKey,
        lastEventAt: now,
        currentTool: toolName ?? prev?.currentTool ?? null,
        error: null,
      }
      this.updateStatus(agentId, entry)
    } else if (state === 'final' || state === 'end') {
      // Agent finished its turn
      const entry: AgentStatusEntry = {
        status: 'idle',
        sessionKey,
        lastEventAt: now,
        currentTool: null,
        error: null,
      }
      this.updateStatus(agentId, entry)
    } else if (state === 'error') {
      const errorMsg =
        typeof p.errorMessage === 'string'
          ? p.errorMessage
          : typeof p.error === 'string'
            ? p.error
            : 'Unknown error'
      const entry: AgentStatusEntry = {
        status: 'error',
        sessionKey,
        lastEventAt: now,
        currentTool: null,
        error: errorMsg,
      }
      this.updateStatus(agentId, entry)
    }
  }

  private updateStatus(agentId: string, entry: AgentStatusEntry): void {
    this.agentStatuses.set(agentId, entry)
    for (const listener of this.listeners) {
      try {
        listener(agentId, entry)
      } catch {}
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
 * OpenClaw may include tool info in delta events when the agent is mid-tool-call.
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
