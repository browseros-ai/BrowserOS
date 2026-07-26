/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { randomUUID } from 'node:crypto'
import { logger } from '../../logger'
import { type AgentSessionId, MAIN_AGENT_SESSION_ID } from '../agent-types'
import type { AgentStreamEvent } from '../types'

export type TurnStatus = 'running' | 'done' | 'error' | 'cancelled'

export interface TurnFrame {
  seq: number
  event: AgentStreamEvent
  createdAt: number
}

export interface ActiveTurnInfo {
  turnId: string
  agentId: string
  sessionId: AgentSessionId
  status: TurnStatus
  lastSeq: number
  startedAt: number
  endedAt?: number
  /** User message that kicked off the turn; null when not captured. */
  prompt: string | null
}

export interface Subscriber {
  push(frame: TurnFrame): void
  end(): void
}

export interface ActiveTurn {
  turnId: string
  agentId: string
  sessionId: AgentSessionId
  status: TurnStatus
  buffer: RingBuffer
  subscribers: Set<Subscriber>
  /** Per-turn AbortController. Aborting cancels the runtime call. */
  abortController: AbortController
  startedAt: number
  endedAt?: number
  retainUntil?: number
  /** User message that kicked off the turn (when known). */
  prompt: string | null
}

const DEFAULT_BUFFER_CAPACITY = 5000
const DEFAULT_RETAIN_AFTER_DONE_MS = 5 * 60 * 1000
const DEFAULT_SWEEP_INTERVAL_MS = 60 * 1000

/**
 * Append-only ring buffer keyed by monotonic `seq`. When capacity is
 * exceeded the oldest frame is dropped (drop-oldest policy) and the
 * `truncated` flag is set so subscribers resuming from a now-evicted seq
 * know to refetch history. The terminal frame (`done`/`error`) is held
 * separately so it's never evicted by overflow.
 */
export class RingBuffer {
  private readonly frames: TurnFrame[] = []
  private readonly capacity: number
  private nextSeq = 0
  private terminal: TurnFrame | null = null
  truncated = false

  constructor(capacity: number = DEFAULT_BUFFER_CAPACITY) {
    this.capacity = capacity
  }

  push(event: AgentStreamEvent): TurnFrame {
    const frame: TurnFrame = {
      seq: this.nextSeq++,
      event,
      createdAt: Date.now(),
    }
    if (event.type === 'done' || event.type === 'error') {
      this.terminal = frame
    }
    this.frames.push(frame)
    if (this.frames.length > this.capacity) {
      this.frames.shift()
      this.truncated = true
    }
    return frame
  }

  /** Frames with seq > fromSeq, plus the terminal frame if not already in the slice. */
  slice(fromSeq: number): TurnFrame[] {
    const live = this.frames.filter((f) => f.seq > fromSeq)
    if (this.terminal && !live.some((f) => f.seq === this.terminal?.seq)) {
      // Terminal might have been evicted by overflow; re-attach it so
      // subscribers always see a terminal if one exists.
      if (this.terminal.seq > fromSeq) live.push(this.terminal)
    }
    return live
  }

  get lastSeq(): number {
    return this.nextSeq - 1
  }

  get length(): number {
    return this.frames.length
  }
}

/**
 * Per-process registry of in-flight agent turns. Decouples the turn
 * lifecycle from any single SSE response: turns keep running when the
 * caller disconnects, and reconnects can replay buffered events to
 * catch up.
 *
 * Lifecycle:
 *   register → run pump that pushes frames → complete/cancel → retain
 *   for `retainAfterDoneMs` so a brief reconnect can still attach →
 *   sweep evicts after retain window.
 */
export class TurnRegistry {
  private readonly turns = new Map<string, ActiveTurn>()
  private readonly retainAfterDoneMs: number
  private readonly sweepIntervalMs: number
  private sweepTimer: ReturnType<typeof setInterval> | null = null

  constructor(
    options: {
      retainAfterDoneMs?: number
      sweepIntervalMs?: number
    } = {},
  ) {
    this.retainAfterDoneMs =
      options.retainAfterDoneMs ?? DEFAULT_RETAIN_AFTER_DONE_MS
    this.sweepIntervalMs = options.sweepIntervalMs ?? DEFAULT_SWEEP_INTERVAL_MS
  }

  /**
   * Register a new turn. The caller is responsible for kicking off the
   * runtime call and pumping its events into `pushEvent` until done.
   */
  register(
    agentId: string,
    sessionId: AgentSessionId = MAIN_AGENT_SESSION_ID,
    options: { prompt?: string | null } = {},
  ): ActiveTurn {
    const turn: ActiveTurn = {
      turnId: randomUUID(),
      agentId,
      sessionId,
      status: 'running',
      buffer: new RingBuffer(),
      subscribers: new Set(),
      abortController: new AbortController(),
      startedAt: Date.now(),
      prompt: options.prompt ?? null,
    }
    this.turns.set(turn.turnId, turn)
    this.ensureSweeper()
    return turn
  }

  get(turnId: string): ActiveTurn | undefined {
    return this.turns.get(turnId)
  }

  /**
   * Active (running) turn for the (agentId, sessionId) pair, if any.
   * Used by route layer for collision detection on POST /chat.
   */
  getActiveFor(
    agentId: string,
    sessionId: AgentSessionId = MAIN_AGENT_SESSION_ID,
  ): ActiveTurn | undefined {
    for (const turn of this.turns.values()) {
      if (
        turn.status === 'running' &&
        turn.agentId === agentId &&
        turn.sessionId === sessionId
      ) {
        return turn
      }
    }
    return undefined
  }

  hasRunning(): boolean {
    for (const turn of this.turns.values()) {
      if (turn.status === 'running') return true
    }
    return false
  }

  describe(turnId: string): ActiveTurnInfo | null {
    const turn = this.turns.get(turnId)
    if (!turn) return null
    return {
      turnId: turn.turnId,
      agentId: turn.agentId,
      sessionId: turn.sessionId,
      status: turn.status,
      lastSeq: turn.buffer.lastSeq,
      startedAt: turn.startedAt,
      endedAt: turn.endedAt,
      prompt: turn.prompt,
    }
  }

  /**
   * Push an event into the turn's buffer and fan out to live
   * subscribers. Terminal events transition status and start the
   * retention window.
   */
  pushEvent(turnId: string, event: AgentStreamEvent): TurnFrame | null {
    const turn = this.turns.get(turnId)
    if (!turn) return null
    if (turn.status !== 'running') return null
    const frame = turn.buffer.push(event)
    for (const sub of turn.subscribers) {
      try {
        sub.push(frame)
      } catch (err) {
        logger.warn('Subscriber push threw; dropping subscriber', {
          turnId,
          error: err instanceof Error ? err.message : String(err),
        })
        turn.subscribers.delete(sub)
      }
    }
    if (event.type === 'done') {
      this.markTerminal(turn, 'done')
    } else if (event.type === 'error') {
      this.markTerminal(turn, 'error')
    }
    return frame
  }

  /**
   * Mark a still-running turn as cancelled and signal its
   * AbortController so the runtime call unwinds. Idempotent.
   */
  cancel(turnId: string, reason?: string): boolean {
    const turn = this.turns.get(turnId)
    if (!turn) return false
    if (turn.status !== 'running') return false
    try {
      turn.abortController.abort(reason ?? 'cancelled')
    } catch {
      // AbortController.abort can throw on Node typing edge cases when
      // signal is already aborted; harmless.
    }
    // We synthesize a terminal `done` with stopReason=cancelled so any
    // subscribers see a clean end. The runtime cancel may also produce
    // an error event later, but the buffer ignores additional pushes
    // once status flips off `running`.
    this.pushSynthetic(turn, {
      type: 'done',
      stopReason: 'cancelled',
      text: reason,
    })
    this.markTerminal(turn, 'cancelled')
    return true
  }

  /**
   * Subscribe to a turn from `fromSeq` (exclusive). Returns a
   * ReadableStream of TurnFrames that completes when the turn does.
   * Cancelling the stream just unregisters the subscriber — the turn
   * keeps running.
   */
  subscribe(
    turnId: string,
    options: { fromSeq?: number; signal?: AbortSignal } = {},
  ): ReadableStream<TurnFrame> | null {
    const turn = this.turns.get(turnId)
    if (!turn) return null
    const fromSeq = options.fromSeq ?? -1

    let queueResolve: ((frame: TurnFrame | null) => void) | null = null
    const pendingFrames: TurnFrame[] = []

    const subscriber: Subscriber = {
      push(frame) {
        if (queueResolve) {
          const r = queueResolve
          queueResolve = null
          r(frame)
        } else {
          pendingFrames.push(frame)
        }
      },
      end() {
        if (queueResolve) {
          const r = queueResolve
          queueResolve = null
          r(null)
        }
      },
    }

    return new ReadableStream<TurnFrame>({
      start: (controller) => {
        // Replay buffered frames first.
        for (const frame of turn.buffer.slice(fromSeq)) {
          controller.enqueue(frame)
        }
        // If the turn already finished, close immediately.
        if (turn.status !== 'running') {
          controller.close()
          return
        }
        turn.subscribers.add(subscriber)
        if (options.signal) {
          if (options.signal.aborted) {
            turn.subscribers.delete(subscriber)
            controller.close()
            return
          }
          options.signal.addEventListener(
            'abort',
            () => {
              turn.subscribers.delete(subscriber)
              try {
                controller.close()
              } catch {
                // Already closed — fine.
              }
            },
            { once: true },
          )
        }
        const pump = async () => {
          try {
            while (true) {
              let nextFrame = pendingFrames.shift()
              while (nextFrame !== undefined) {
                controller.enqueue(nextFrame)
                nextFrame = pendingFrames.shift()
              }
              if (turn.status !== 'running' && pendingFrames.length === 0) {
                break
              }
              const next = await new Promise<TurnFrame | null>((resolve) => {
                queueResolve = resolve
              })
              if (next === null) break
              controller.enqueue(next)
            }
            controller.close()
          } catch (err) {
            try {
              controller.error(err)
            } catch {
              // Already errored — fine.
            }
          } finally {
            turn.subscribers.delete(subscriber)
          }
        }
        void pump()
      },
      cancel: () => {
        turn.subscribers.delete(subscriber)
        subscriber.end()
      },
    })
  }

  /**
   * Periodic eviction of turns that have been terminal past
   * `retainAfterDoneMs`. Lazy — only runs while the registry has
   * entries.
   */
  sweep(now: number = Date.now()): void {
    for (const [turnId, turn] of this.turns) {
      if (turn.status === 'running') continue
      if (turn.retainUntil != null && now >= turn.retainUntil) {
        this.turns.delete(turnId)
      }
    }
    if (this.turns.size === 0 && this.sweepTimer) {
      clearInterval(this.sweepTimer)
      this.sweepTimer = null
    }
  }

  /** For tests. */
  size(): number {
    return this.turns.size
  }

  /** For tests / shutdown. Stops the sweep timer; does not clear turns. */
  stopSweeper(): void {
    if (this.sweepTimer) {
      clearInterval(this.sweepTimer)
      this.sweepTimer = null
    }
  }

  private markTerminal(turn: ActiveTurn, status: TurnStatus): void {
    if (turn.status !== 'running') return
    turn.status = status
    turn.endedAt = Date.now()
    turn.retainUntil = turn.endedAt + this.retainAfterDoneMs
    for (const sub of turn.subscribers) sub.end()
    turn.subscribers.clear()
  }

  private pushSynthetic(turn: ActiveTurn, event: AgentStreamEvent): void {
    if (turn.status !== 'running') return
    const frame = turn.buffer.push(event)
    for (const sub of turn.subscribers) {
      try {
        sub.push(frame)
      } catch {
        // ignore
      }
    }
  }

  private ensureSweeper(): void {
    if (this.sweepTimer) return
    this.sweepTimer = setInterval(() => this.sweep(), this.sweepIntervalMs)
    // Don't keep the process alive on the timer alone.
    if (typeof this.sweepTimer === 'object' && 'unref' in this.sweepTimer) {
      this.sweepTimer.unref()
    }
  }
}
