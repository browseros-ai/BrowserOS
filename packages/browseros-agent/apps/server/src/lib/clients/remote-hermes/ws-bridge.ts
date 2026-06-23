/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Persistent WebSocket bridge to the agent-control-worker, plus the
 * dispatch logic that routes worker-originated `rpc.request` frames to
 * the laptop's local BrowserOS MCP server.
 *
 * The class is shape-aware of three invariants:
 *  1. At most one OPEN socket per process. Concurrent `ensureOpen()`
 *     callers share the same `openingPromise`.
 *  2. Never close while a turn is in flight. `withTurn(fn)` ref-counts.
 *  3. Activity-based idle close. The sweep timer only acts when
 *     `inflightTurns === 0` AND we've been quiet for IDLE_CLOSE_MS.
 */

import ReconnectingWebSocket from 'partysocket/ws'
import { logger } from '../../logger'
import {
  CLOSE_CODE_REPLACED,
  IDLE_CLOSE_MS,
  IDLE_SWEEP_INTERVAL_MS,
  MAX_ENQUEUED_MESSAGES,
  OPEN_DEADLINE_MS,
  PING_INTERVAL_MS,
  PONG_TIMEOUT_MS,
  TURN_REFCOUNT_GUARD_MS,
} from './constants'
import { encodeFrame, PING_FRAME, parseFrame } from './frames'
import type { RemoteHermesClient } from './remote-hermes-client'
import { dispatchRpcRequest } from './rpc-router'

export interface WsBridgeDeps {
  client: RemoteHermesClient
  resolveLocalMcpUrl(server: string): string | null
}

export type SocketState = 'closed' | 'connecting' | 'open'

const MODULE = 'remote-hermes'

export class WsBridge {
  private socket: ReconnectingWebSocket | null = null
  private openingPromise: Promise<void> | null = null
  private inflightTurns = 0
  private lastActivityAt = 0
  private idleSweepTimer: ReturnType<typeof setInterval> | null = null
  private pingTimer: ReturnType<typeof setInterval> | null = null
  private lastPongAt = 0
  private state: SocketState = 'closed'

  constructor(private readonly deps: WsBridgeDeps) {}

  async ensureOpen(): Promise<void> {
    if (this.state === 'open') return
    if (this.openingPromise) return this.openingPromise
    this.openingPromise = this.doOpen().finally(() => {
      this.openingPromise = null
    })
    return this.openingPromise
  }

  async withTurn<T>(fn: () => Promise<T>): Promise<T> {
    await this.ensureOpen()
    this.inflightTurns++
    this.touch()
    const guard = setTimeout(() => {
      if (this.inflightTurns > 0) {
        this.inflightTurns = Math.max(0, this.inflightTurns - 1)
        logger.warn('Remote Hermes refcount safety-belt fired', {
          module: MODULE,
        })
      }
    }, TURN_REFCOUNT_GUARD_MS)
    try {
      return await fn()
    } finally {
      clearTimeout(guard)
      this.inflightTurns = Math.max(0, this.inflightTurns - 1)
      this.touch()
    }
  }

  /** Called from Application.shutdown(). */
  close(): void {
    this.stopPings()
    this.stopIdleSweep()
    try {
      this.socket?.close()
    } catch {
      // already closed
    }
    this.socket = null
    this.state = 'closed'
  }

  /** Exposed for diagnostics. */
  snapshot(): {
    state: SocketState
    inflightTurns: number
    lastActivityAt: number
  } {
    return {
      state: this.state,
      inflightTurns: this.inflightTurns,
      lastActivityAt: this.lastActivityAt,
    }
  }

  private async doOpen(): Promise<void> {
    // If a previous doOpen call timed out at OPEN_DEADLINE_MS but the
    // underlying socket is still attempting to connect (partysocket's
    // connectionTimeout is longer than ours), close it now. Otherwise
    // the stale socket can fire 'open' later, flip state on the new
    // socket's behalf, and start a parallel set of pings + idle sweep.
    if (this.socket) {
      try {
        this.socket.close()
      } catch {
        // already closed
      }
      this.socket = null
    }
    this.state = 'connecting'
    const sock = new ReconnectingWebSocket(
      this.deps.client.wsUrl,
      () => this.deps.client.wsSubprotocol(),
      {
        // partysocket has a flat-delay bug despite README claiming jitter
        // (packages/partysocket/src/ws.ts:127 vs README:174). Lockstep
        // retries across clients after a CF blip — add per-instance
        // jitter ourselves.
        minReconnectionDelay: 1_000 + Math.random() * 2_000,
        maxReconnectionDelay: 30_000,
        reconnectionDelayGrowFactor: 1.5,
        connectionTimeout: 15_000,
        minUptime: 10_000,
        maxEnqueuedMessages: MAX_ENQUEUED_MESSAGES,
      },
    )
    // Default 'blob' fights node-style WS impls. arraybuffer is safe everywhere.
    sock.binaryType = 'arraybuffer'

    // Every handler checks `this.socket === sock` so an event from an
    // orphaned previous socket (one we closed above) can never mutate
    // shared bridge state.
    sock.addEventListener('open', () => {
      if (this.socket !== sock) return
      this.state = 'open'
      this.touch()
      this.startPings()
      this.startIdleSweep()
      logger.info('Remote Hermes WS open', {
        module: MODULE,
        wsUrl: this.deps.client.wsUrl,
      })
    })
    sock.addEventListener('close', (ev) => {
      if (this.socket !== sock) return
      this.stopPings()
      const replaced = (ev as CloseEvent).code === CLOSE_CODE_REPLACED
      if (replaced) {
        logger.info('Remote Hermes WS replaced by server; not reconnecting', {
          module: MODULE,
        })
        try {
          sock.close()
        } catch {
          // already closed
        }
        this.socket = null
        this.state = 'closed'
        return
      }
      this.state = 'connecting'
    })
    sock.addEventListener('error', () => {
      if (this.socket !== sock) return
      // partysocket fires reconnect on its own; just record activity so
      // idle sweep doesn't fire mid-reconnect.
      this.touch()
    })
    sock.addEventListener('message', (ev) => {
      if (this.socket !== sock) return
      this.touch()
      void this.onMessage(ev as MessageEvent).catch((err) =>
        logger.warn('Remote Hermes onMessage error', {
          module: MODULE,
          err: err instanceof Error ? err.message : String(err),
        }),
      )
    })

    this.socket = sock

    await new Promise<void>((resolve, reject) => {
      const t = setTimeout(() => {
        reject(new Error(`ws open timed out after ${OPEN_DEADLINE_MS}ms`))
      }, OPEN_DEADLINE_MS)
      const onOpen = () => {
        clearTimeout(t)
        sock.removeEventListener('open', onOpen)
        resolve()
      }
      sock.addEventListener('open', onOpen)
    })
  }

  private async onMessage(ev: MessageEvent): Promise<void> {
    const raw =
      typeof ev.data === 'string'
        ? ev.data
        : ev.data instanceof ArrayBuffer
          ? new TextDecoder().decode(ev.data)
          : String(ev.data)
    const frame = parseFrame(raw)
    if (!frame) return
    if (frame.type === 'pong') {
      this.lastPongAt = Date.now()
      return
    }
    if (frame.type === 'ping' || frame.type === 'rpc.response') return
    const reply = await dispatchRpcRequest(frame, {
      resolveBaseUrl: this.deps.resolveLocalMcpUrl,
      // v1: use the install's browserosId as a stable per-install scope.
      // Per-conversation isolation requires worker-side propagation of
      // threadId on the rpc.request frame (Open Item #2 in the design
      // doc).
      scopeId: this.deps.client.browserosId,
      agentId: 'remote-hermes',
    })
    try {
      this.socket?.send(encodeFrame(reply))
      this.touch()
    } catch (err) {
      logger.warn('Remote Hermes failed to send rpc.response', {
        module: MODULE,
        err: err instanceof Error ? err.message : String(err),
      })
    }
  }

  private touch(): void {
    this.lastActivityAt = Date.now()
  }

  private startPings(): void {
    this.stopPings()
    this.lastPongAt = Date.now()
    this.pingTimer = setInterval(() => {
      if (Date.now() - this.lastPongAt > PONG_TIMEOUT_MS) {
        logger.warn('Remote Hermes WS pong timeout; forcing reconnect', {
          module: MODULE,
          ageMs: Date.now() - this.lastPongAt,
        })
        try {
          this.socket?.reconnect()
        } catch {
          // close handler will reschedule
        }
        return
      }
      try {
        // CF's setWebSocketAutoResponse intercepts this literal and
        // replies {"type":"pong"} without waking the DO. The static
        // literal matters.
        this.socket?.send(PING_FRAME)
      } catch {
        // close handler will reconnect
      }
    }, PING_INTERVAL_MS)
    this.pingTimer.unref?.()
  }

  private stopPings(): void {
    if (this.pingTimer) clearInterval(this.pingTimer)
    this.pingTimer = null
  }

  private startIdleSweep(): void {
    if (this.idleSweepTimer) return
    this.idleSweepTimer = setInterval(
      () => this.idleSweep(),
      IDLE_SWEEP_INTERVAL_MS,
    )
    this.idleSweepTimer.unref?.()
  }

  private stopIdleSweep(): void {
    if (this.idleSweepTimer) clearInterval(this.idleSweepTimer)
    this.idleSweepTimer = null
  }

  private idleSweep(): void {
    if (!this.socket) return
    if (this.inflightTurns > 0) return
    if (Date.now() - this.lastActivityAt < IDLE_CLOSE_MS) return
    logger.info('Remote Hermes WS idle close', { module: MODULE })
    try {
      this.socket.close(1000, 'idle')
    } catch {
      // already closed
    }
    this.socket = null
    this.state = 'closed'
    this.stopPings()
    this.stopIdleSweep()
  }
}
