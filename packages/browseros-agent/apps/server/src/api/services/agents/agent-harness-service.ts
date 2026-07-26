/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  AcpxRuntime,
  unwrapBrowserosAcpUserMessage,
} from '../../../lib/agents/acpx/runtime'
import {
  type AgentDefinition,
  type AgentSessionId,
  MAIN_AGENT_SESSION_ID,
} from '../../../lib/agents/agent-types'
import type {
  AgentStore,
  CreateAgentInput,
} from '../../../lib/agents/storage/agent-store'
import { DbAgentStore } from '../../../lib/agents/storage/db-agent-store'
import {
  FileMessageQueue,
  type QueuedMessage,
  type QueuedMessageAttachment,
} from '../../../lib/agents/storage/message-queue'
import {
  type ActiveTurnInfo,
  type TurnFrame,
  TurnRegistry,
} from '../../../lib/agents/turns/active-turn-registry'

export {
  MessageQueueFullError,
  type QueuedMessage,
  type QueuedMessageAttachment,
} from '../../../lib/agents/storage/message-queue'

import { join } from 'node:path'
import type {
  AgentHistoryPage,
  AgentRowSnapshot,
  AgentRuntime,
  AgentStreamEvent,
} from '../../../lib/agents/types'
import { getBrowserosDir } from '../../../lib/browseros-dir'
import { logger } from '../../../lib/logger'

export type AgentLiveness = 'working' | 'idle' | 'asleep' | 'error'

type SessionActivity = {
  sessionId: AgentSessionId
  status: 'working' | 'error'
  lastEventAt: number
  lastError?: string
}

export interface AgentDefinitionWithActivity extends AgentDefinition {
  status: AgentLiveness
  lastUsedAt: number | null
  /** First non-blank line of the most recent user message; null if none. */
  lastUserMessage: string | null
  /** Working directory the agent runs in; null when no session record yet. */
  cwd: string | null
  /** Cumulative + 7-day rolling token usage; null when no record. */
  tokens: AgentRowSnapshot['tokens']
  /**
   * Last 14 days of completed turns, oldest → newest. Zero-filled in
   * this release until the activity ledger ships in a follow-up.
   */
  turnsByDay: number[]
  /** Same shape as `turnsByDay`; counts of failed turns. */
  failedByDay: number[]
  /** Last error message when status === 'error'; null otherwise. */
  lastError: string | null
  lastErrorAt: number | null
  /** Most recent persisted session for this agent; null until first use. */
  latestSessionId: AgentSessionId | null
  /** When non-null, an in-flight turn this row can be resumed from. */
  activeTurnId: string | null
  /** Persistent FIFO queue of messages waiting to run for this agent. */
  queue: QueuedMessage[]
}

const SPARKLINE_DAYS = 14
const ZERO_BUCKETS = (): number[] =>
  Array.from({ length: SPARKLINE_DAYS }, () => 0)

/**
 * `idle` downgrades to `asleep` after this many ms of no activity. Read at
 * enrichment time; no timer cleanup necessary.
 */
const ASLEEP_THRESHOLD_MS = 15 * 60 * 1000

function activityKey(agentId: string, sessionId: AgentSessionId): string {
  return `${agentId}\u0000${sessionId}`
}

/**
 * Per-turn event the harness emits to subscribers. Lets services that
 * want to track liveness for a specific adapter react to the same
 * stream the chat panel sees.
 */
export type TurnLifecycleEvent =
  | { type: 'turn_started' }
  | { type: 'turn_event'; event: AgentStreamEvent }
  | { type: 'turn_ended'; error?: string }

export type TurnLifecycleListener = (
  agent: {
    id: string
    adapter: AgentDefinition['adapter']
    sessionKey: string
  },
  event: TurnLifecycleEvent,
) => void

export class AgentHarnessService {
  private readonly agentStore: AgentStore
  private readonly runtime: AgentRuntime
  private readonly browserosDir: string
  private readonly turnRegistry: TurnRegistry
  private readonly messageQueue: FileMessageQueue
  private readonly turnLifecycleListeners = new Set<TurnLifecycleListener>()
  // In-memory liveness tracker. Lost on server restart (acceptable —
  // `lastUsedAt` survives via the acpx session record's `lastUsedAt`,
  // and an idle/asleep agent post-restart will read fine from the
  // record's timestamp without ever flipping to `working`).
  private readonly activity = new Map<string, SessionActivity>()

  constructor(
    deps: {
      agentStore?: AgentStore
      runtime?: AgentRuntime
      browserosDir?: string
      resourcesDir?: string
      browserosServerPort?: number
      turnRegistry?: TurnRegistry
      messageQueue?: FileMessageQueue
    } = {},
  ) {
    this.browserosDir = deps.browserosDir ?? getBrowserosDir()
    this.agentStore = deps.agentStore ?? new DbAgentStore()
    this.runtime =
      deps.runtime ??
      new AcpxRuntime({
        browserosDir: this.browserosDir,
        resourcesDir: deps.resourcesDir,
        browserosServerPort: deps.browserosServerPort,
      })
    this.turnRegistry = deps.turnRegistry ?? new TurnRegistry()
    this.messageQueue =
      deps.messageQueue ??
      new FileMessageQueue({
        filePath: join(
          this.browserosDir,
          'agents',
          'harness',
          'message-queues.json',
        ),
      })
    // Drain any agents whose queue file survived a restart. The check
    // for `getActiveFor` inside `maybeStartNextFromQueue` guards
    // against double-firing if the in-memory turn registry happens to
    // have something (it won't post-restart, but the guard is cheap).
    void this.drainOnBoot()
  }

  private async drainOnBoot(): Promise<void> {
    try {
      const pending = await this.messageQueue.agentsWithPendingMessages()
      for (const agentId of pending) {
        void this.maybeStartNextFromQueue(agentId)
      }
    } catch (err) {
      logger.warn('Message queue boot drain failed', {
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  async listAgents(): Promise<AgentDefinition[]> {
    return this.agentStore.list()
  }

  /**
   * Same shape as `listAgents()` but every record is enriched with the
   * current liveness state and `lastUsedAt`. Liveness is read from the
   * in-memory activity tracker — which only knows about turns that
   * went through this process — falling back to a timestamp-derived
   * `idle`/`asleep` from the acpx session record's `lastUsedAt`.
   */
  async listAgentsWithActivity(): Promise<AgentDefinitionWithActivity[]> {
    const agents = await this.listAgents()
    const [snapshots, queueSnapshot] = await Promise.all([
      this.collectRowSnapshots(agents),
      this.messageQueue.snapshotAll(),
    ])
    const now = Date.now()
    return agents.map((agent) => {
      const snapshot = snapshots.get(agent.id) ?? null
      const liveLatest = this.getLatestActivity(agent.id)
      const liveWins =
        liveLatest != null &&
        (!snapshot?.lastUsedAt || liveLatest.lastEventAt >= snapshot.lastUsedAt)
      const latestSessionId = liveWins
        ? liveLatest.sessionId
        : (snapshot?.sessionId ?? null)
      const live = latestSessionId
        ? this.activity.get(activityKey(agent.id, latestSessionId))
        : liveLatest
      const lastUsedAt = liveWins
        ? liveLatest.lastEventAt
        : (snapshot?.lastUsedAt ?? null)
      const activeTurn = latestSessionId
        ? this.turnRegistry.getActiveFor(agent.id, latestSessionId)
        : null
      return {
        ...agent,
        pinned: agent.pinned ?? false,
        status: deriveStatus(live, lastUsedAt, now),
        lastUsedAt,
        lastUserMessage:
          activeTurn?.prompt ?? snapshot?.lastUserMessage ?? null,
        cwd: snapshot?.cwd ?? null,
        tokens: snapshot?.tokens ?? null,
        turnsByDay: ZERO_BUCKETS(),
        failedByDay: ZERO_BUCKETS(),
        lastError: live?.status === 'error' ? (live.lastError ?? null) : null,
        lastErrorAt:
          live?.status === 'error' ? (live.lastEventAt ?? null) : null,
        latestSessionId,
        activeTurnId: activeTurn?.turnId ?? null,
        queue: queueSnapshot[agent.id] ?? [],
      }
    })
  }

  /**
   * Pull one snapshot per agent in parallel. Falls back to a
   * lastUsedAt-only snapshot when the runtime doesn't implement
   * `getRowSnapshot` (test fakes, future runtimes), so the listing
   * stays robust during migration.
   */
  private async collectRowSnapshots(
    agents: AgentDefinition[],
  ): Promise<Map<string, AgentRowSnapshot>> {
    const out = new Map<string, AgentRowSnapshot>()
    await Promise.all(
      agents.map(async (agent) => {
        try {
          const snapshot = await this.fetchRowSnapshot(agent)
          if (snapshot) out.set(agent.id, snapshot)
        } catch {
          // No record yet — treat as never-used.
        }
      }),
    )
    return out
  }

  private async fetchRowSnapshot(
    agent: AgentDefinition,
  ): Promise<AgentRowSnapshot | null> {
    if (typeof this.runtime.getLatestRowSnapshot === 'function') {
      return this.runtime.getLatestRowSnapshot(agent)
    }
    if (typeof this.runtime.getRowSnapshot === 'function') {
      const snapshot = await this.runtime.getRowSnapshot({
        agent,
        sessionId: MAIN_AGENT_SESSION_ID,
      })
      return snapshot
        ? {
            ...snapshot,
            sessionId: snapshot.sessionId ?? MAIN_AGENT_SESSION_ID,
          }
        : null
    }
    // Legacy fallback: derive only `lastUsedAt` from the history page.
    const page = await this.runtime.getHistory({
      agent,
      sessionId: MAIN_AGENT_SESSION_ID,
    })
    const last = page.items.at(-1)?.createdAt
    if (typeof last !== 'number' || !Number.isFinite(last)) return null
    return {
      sessionId: MAIN_AGENT_SESSION_ID,
      cwd: null,
      lastUsedAt: last,
      lastUserMessage: null,
      tokens: null,
    }
  }

  private getLatestActivity(agentId: string): SessionActivity | undefined {
    const prefix = `${agentId}\u0000`
    let latest: SessionActivity | undefined
    for (const [key, entry] of this.activity.entries()) {
      if (!key.startsWith(prefix)) continue
      if (!latest || entry.lastEventAt > latest.lastEventAt) latest = entry
    }
    return latest
  }

  /**
   * Subscribe to turn lifecycle events for every running agent. Returns
   * an unsubscribe function. Listeners are best-effort: a throwing
   * listener does not break the turn.
   */
  onTurnLifecycle(listener: TurnLifecycleListener): () => void {
    this.turnLifecycleListeners.add(listener)
    return () => this.turnLifecycleListeners.delete(listener)
  }

  private emitTurnLifecycle(
    agent: AgentDefinition,
    event: TurnLifecycleEvent,
  ): void {
    if (this.turnLifecycleListeners.size === 0) return
    const summary = {
      id: agent.id,
      adapter: agent.adapter,
      sessionKey: agent.sessionKey,
    }
    for (const listener of this.turnLifecycleListeners) {
      try {
        listener(summary, event)
      } catch (err) {
        logger.warn('Turn lifecycle listener threw', {
          agentId: agent.id,
          eventType: event.type,
          error: err instanceof Error ? err.message : String(err),
        })
      }
    }
  }

  /** Mark an agent session as actively running a turn. */
  notifyTurnStarted(
    agentId: string,
    sessionId: AgentSessionId = MAIN_AGENT_SESSION_ID,
  ): void {
    this.activity.set(activityKey(agentId, sessionId), {
      sessionId,
      status: 'working',
      lastEventAt: Date.now(),
    })
  }

  /** Clear the session working flag. `error` keeps that session badged as needing attention. */
  notifyTurnEnded(
    agentId: string,
    sessionId: AgentSessionId = MAIN_AGENT_SESSION_ID,
    outcome: { ok: boolean; error?: string } = { ok: true },
  ): void {
    const key = activityKey(agentId, sessionId)
    if (!outcome.ok) {
      this.activity.set(key, {
        sessionId,
        status: 'error',
        lastEventAt: Date.now(),
        lastError: outcome.error,
      })
    } else {
      // Successful turn — clear working only when this same session has no
      // remaining running turn.
      if (this.turnRegistry.getActiveFor(agentId, sessionId)) {
        this.activity.set(key, {
          sessionId,
          status: 'working',
          lastEventAt: Date.now(),
        })
      } else {
        this.activity.delete(key)
      }
    }
    // The queue drain runs on every turn-end (success or failure) so
    // a queued message is the next thing to run. Fire-and-forget; any
    // failure inside `maybeStartNextFromQueue` requeues the message
    // and logs.
    void this.maybeStartNextFromQueue(agentId)
  }

  /**
   * Pop the oldest queued message for `agentId` and start a turn from
   * it. Fires from `notifyTurnEnded` (covers natural completion +
   * cancel) and on server boot to drain queue files that survived a
   * restart. No-ops when the queue is empty or another turn is
   * already running for the agent.
   */
  private async maybeStartNextFromQueue(agentId: string): Promise<void> {
    const next = await this.messageQueue.popOldest(agentId)
    if (!next) return
    const sessionId = next.sessionId ?? MAIN_AGENT_SESSION_ID
    // Race guard: a turn may have started between `popOldest` and now
    // (e.g. the user typed and clicked Send directly between cancel
    // and the drain). Put the message back at the head and let the
    // next turn-end retry.
    if (this.turnRegistry.getActiveFor(agentId, sessionId)) {
      await this.messageQueue.pushFront(agentId, next)
      return
    }
    try {
      await this.startTurn({
        agentId,
        sessionId,
        message: next.message,
        attachments: next.attachments,
      })
    } catch (err) {
      logger.warn('Queue drain failed; requeued message', {
        agentId,
        queuedId: next.id,
        error: err instanceof Error ? err.message : String(err),
      })
      try {
        await this.messageQueue.pushFront(agentId, next)
      } catch (requeueErr) {
        logger.error('Queue requeue after drain failure also failed', {
          agentId,
          queuedId: next.id,
          error:
            requeueErr instanceof Error
              ? requeueErr.message
              : String(requeueErr),
        })
      }
    }
  }

  /**
   * Append a message to the agent's queue. Returns the new queued
   * record. Throws `UnknownAgentError` for unknown agents and
   * `MessageQueueFullError` when the per-agent cap is reached.
   */
  async enqueueMessage(input: {
    agentId: string
    sessionId?: AgentSessionId
    message: string
    attachments?: ReadonlyArray<QueuedMessageAttachment>
  }): Promise<QueuedMessage> {
    const agent = await this.requireAgent(input.agentId)
    const queued = await this.messageQueue.append(agent.id, {
      sessionId: input.sessionId,
      message: input.message,
      attachments: input.attachments,
    })
    // Defensive drain: if the agent has no active turn at enqueue
    // time (e.g. the user enqueued during the brief window between
    // turns), pop it back off and start it directly. Avoids the
    // queue sitting idle while the agent is also idle.
    if (
      !this.turnRegistry.getActiveFor(
        agent.id,
        input.sessionId ?? MAIN_AGENT_SESSION_ID,
      )
    ) {
      void this.maybeStartNextFromQueue(agent.id)
    }
    return queued
  }

  /**
   * Remove a queued message. Returns true if the message was
   * removed, false if the agent or message was unknown.
   */
  async removeQueuedMessage(input: {
    agentId: string
    messageId: string
  }): Promise<boolean> {
    return this.messageQueue.remove(input.agentId, input.messageId)
  }

  async listQueuedMessages(agentId: string): Promise<QueuedMessage[]> {
    return this.messageQueue.list(agentId)
  }

  async createAgent(input: CreateAgentInput): Promise<AgentDefinition> {
    return this.agentStore.create(input)
  }

  async deleteAgent(agentId: string): Promise<boolean> {
    const agent = await this.agentStore.get(agentId)
    if (!agent) return false
    return this.agentStore.delete(agentId)
  }

  /**
   * Apply a partial update to an agent record. Currently used by the
   * pin-toggle mutation; rename will land here too. Returns null if
   * the agent doesn't exist; throws on validation failure so the
   * route layer can surface a 400.
   */
  async updateAgent(
    agentId: string,
    patch: { name?: string; pinned?: boolean },
  ): Promise<AgentDefinition | null> {
    if (patch.name !== undefined) {
      const trimmed = patch.name.trim()
      if (!trimmed) {
        throw new InvalidAgentUpdateError('Name is required')
      }
      // Mirror the create-time validation for length consistency.
      const { AGENT_HARNESS_LIMITS } = await import(
        '@browseros/shared/constants/limits'
      )
      if (trimmed.length > AGENT_HARNESS_LIMITS.AGENT_NAME_MAX_CHARS) {
        throw new InvalidAgentUpdateError(
          `Name must be ${AGENT_HARNESS_LIMITS.AGENT_NAME_MAX_CHARS} characters or fewer`,
        )
      }
      patch = { ...patch, name: trimmed }
    }
    return this.agentStore.update(agentId, patch)
  }

  getAgent(agentId: string): Promise<AgentDefinition | null> {
    return this.agentStore.get(agentId)
  }

  async getHistory(
    agentId: string,
    sessionId: AgentSessionId = MAIN_AGENT_SESSION_ID,
  ): Promise<AgentHistoryPage> {
    const agent = await this.requireAgent(agentId)
    return this.runtime.getHistory({ agent, sessionId })
  }

  /**
   * Kick off a new agent turn that survives the caller's HTTP lifetime.
   * Events are pushed into a per-turn buffer; the returned `frames`
   * stream is a *subscription* (replays from seq 0). Closing the stream
   * just unsubscribes; the turn keeps running until terminal or
   * cancelled. Throws `TurnAlreadyActiveError` if the agent is already
   * mid-turn — the route layer maps that to 409.
   */
  async startTurn(input: {
    agentId: string
    sessionId?: AgentSessionId
    message: string
    attachments?: ReadonlyArray<{ mediaType: string; data: string }>
    cwd?: string
  }): Promise<{ turnId: string; frames: ReadableStream<TurnFrame> }> {
    const agent = await this.requireAgent(input.agentId)
    const sessionId = input.sessionId ?? MAIN_AGENT_SESSION_ID

    const existing = this.turnRegistry.getActiveFor(agent.id, sessionId)
    if (existing) {
      throw new TurnAlreadyActiveError(agent.id, existing.turnId)
    }

    const turn = this.turnRegistry.register(agent.id, sessionId, {
      prompt: input.message,
    })
    this.notifyTurnStarted(agent.id, sessionId)
    this.emitTurnLifecycle(agent, { type: 'turn_started' })

    // Kick off the runtime call in the background. The per-turn
    // AbortController — NOT the HTTP request signal — is what cancels
    // the runtime call. This is the core decoupling that lets turns
    // outlive their initiating HTTP request.
    void this.runDetachedTurn(turn.turnId, agent, input)

    const frames = this.turnRegistry.subscribe(turn.turnId, { fromSeq: -1 })
    if (!frames) {
      // Should be impossible — register just put it in the registry —
      // but keep the type narrow.
      throw new Error('Turn registration race')
    }
    return { turnId: turn.turnId, frames }
  }

  /**
   * Attach to an existing turn. Resumes by replaying buffered frames
   * with seq > `lastSeq`, then tails new ones. Returns null if the
   * turn is unknown (e.g. never existed, or its retain window expired).
   */
  attachTurn(input: {
    turnId: string
    lastSeq?: number
  }): ReadableStream<TurnFrame> | null {
    return this.turnRegistry.subscribe(input.turnId, {
      fromSeq: input.lastSeq ?? -1,
    })
  }

  /**
   * Active turn for the (agentId, sessionId) pair, if any. Used by the
   * UI on mount to discover an in-flight turn it should attach to
   * instead of starting a new one.
   */
  getActiveTurn(
    agentId: string,
    sessionId: AgentSessionId = MAIN_AGENT_SESSION_ID,
  ): ActiveTurnInfo | null {
    const turn = this.turnRegistry.getActiveFor(agentId, sessionId)
    if (!turn) return null
    const info = this.turnRegistry.describe(turn.turnId)
    if (!info?.prompt) return info
    // Chat UIs that attach to an in-flight turn render this prompt as the
    // user bubble (new-tab agent view). Strip the browser-context /
    // <USER_QUERY> scaffolding so it shows only the user's question —
    // same read-time unwrap getHistory applies.
    return { ...info, prompt: unwrapBrowserosAcpUserMessage(info.prompt) }
  }

  /**
   * Cancel an active turn. Idempotent — returns true on the first
   * successful cancel, false if the turn doesn't exist or already
   * finished.
   */
  cancelTurn(input: {
    agentId: string
    sessionId?: AgentSessionId
    turnId?: string
    reason?: string
  }): boolean {
    const turnId =
      input.turnId ??
      this.turnRegistry.getActiveFor(
        input.agentId,
        input.sessionId ?? MAIN_AGENT_SESSION_ID,
      )?.turnId
    if (!turnId) return false
    return this.turnRegistry.cancel(turnId, input.reason)
  }

  /**
   * Back-compat wrapper for the old `send` signature. Returns a stream
   * of `AgentStreamEvent` (not `TurnFrame`), so legacy callers/tests
   * keep working. Internally goes through the registry so liveness and
   * resilience semantics still apply. Drops `signal` — turns now own
   * their own AbortController.
   */
  async send(input: {
    agentId: string
    sessionId?: AgentSessionId
    message: string
    attachments?: ReadonlyArray<{ mediaType: string; data: string }>
    cwd?: string
    signal?: AbortSignal
  }): Promise<ReadableStream<AgentStreamEvent>> {
    const { frames } = await this.startTurn(input)
    return frames.pipeThrough(
      new TransformStream<TurnFrame, AgentStreamEvent>({
        transform(frame, controller) {
          controller.enqueue(frame.event)
        },
      }),
    )
  }

  /**
   * Background pump: drives the runtime call, fans events into the
   * registry, and retires the turn on terminal/error/cancel. Never
   * throws to its caller — all failures land as `error` frames.
   */
  private async runDetachedTurn(
    turnId: string,
    agent: AgentDefinition,
    input: {
      message: string
      attachments?: ReadonlyArray<{ mediaType: string; data: string }>
      cwd?: string
    },
  ): Promise<void> {
    const turn = this.turnRegistry.get(turnId)
    if (!turn) return
    const sessionId = turn.sessionId
    let lastErrorMessage: string | undefined

    try {
      const upstream = await this.runtime.send({
        agent,
        sessionId,
        sessionKey: agent.sessionKey,
        message: input.message,
        attachments: input.attachments,
        permissionMode: agent.permissionMode,
        cwd: input.cwd,
        signal: turn.abortController.signal,
      })
      const reader = upstream.getReader()
      try {
        while (true) {
          const { done, value } = await reader.read()
          if (done) break
          if (value.type === 'error') lastErrorMessage = value.message
          this.turnRegistry.pushEvent(turnId, value)
          this.emitTurnLifecycle(agent, { type: 'turn_event', event: value })
        }
      } finally {
        try {
          reader.releaseLock()
        } catch {
          // ignore
        }
      }
      // Synthesize a terminal `done` if the upstream finished without
      // emitting one (defensive — runtime is supposed to, but our
      // resilience contract requires every subscriber to see a
      // terminal frame).
      const refreshed = this.turnRegistry.get(turnId)
      if (refreshed?.status === 'running') {
        if (lastErrorMessage !== undefined) {
          this.turnRegistry.pushEvent(turnId, {
            type: 'error',
            message: lastErrorMessage,
          })
        } else {
          this.turnRegistry.pushEvent(turnId, {
            type: 'done',
            stopReason: 'end_turn',
          })
        }
      }
    } catch (err) {
      lastErrorMessage = err instanceof Error ? err.message : String(err)
      const refreshed = this.turnRegistry.get(turnId)
      if (refreshed?.status === 'running') {
        this.turnRegistry.pushEvent(turnId, {
          type: 'error',
          message: lastErrorMessage,
        })
      }
    } finally {
      this.notifyTurnEnded(agent.id, sessionId, {
        ok: lastErrorMessage === undefined,
        error: lastErrorMessage,
      })
      this.emitTurnLifecycle(agent, {
        type: 'turn_ended',
        error: lastErrorMessage,
      })
    }
  }

  private async requireAgent(agentId: string): Promise<AgentDefinition> {
    const agent = await this.agentStore.get(agentId)
    if (!agent) {
      throw new UnknownAgentError(agentId)
    }
    return agent
  }
}

/**
 * Pure derivation: in-memory activity tracker wins; otherwise we fall
 * back to a timestamp-only judgment. Never-used agents resolve to
 * `idle` so the UI doesn't render them as `asleep` (asleep implies
 * "was active, went quiet").
 */
function deriveStatus(
  live: { status: 'working' | 'error'; lastEventAt: number } | undefined,
  lastUsedAt: number | null,
  now: number,
): AgentLiveness {
  if (live?.status === 'working') return 'working'
  if (live?.status === 'error') return 'error'
  if (lastUsedAt == null) return 'idle'
  return now - lastUsedAt > ASLEEP_THRESHOLD_MS ? 'asleep' : 'idle'
}

export class UnknownAgentError extends Error {
  constructor(readonly agentId: string) {
    super(`Unknown agent: ${agentId}`)
    this.name = 'UnknownAgentError'
  }
}

/**
 * Thrown when an `updateAgent` call carries a payload that fails
 * validation (e.g., empty/oversized name). Route layer maps to 400.
 */
export class InvalidAgentUpdateError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'InvalidAgentUpdateError'
  }
}

/**
 * Thrown when `startTurn` is called for an agent that already has an
 * in-flight turn. The route layer maps this to 409 + the existing
 * `turnId` so the client can attach instead.
 */
export class TurnAlreadyActiveError extends Error {
  constructor(
    readonly agentId: string,
    readonly turnId: string,
  ) {
    super(`Agent ${agentId} already has an active turn (${turnId})`)
    this.name = 'TurnAlreadyActiveError'
  }
}
