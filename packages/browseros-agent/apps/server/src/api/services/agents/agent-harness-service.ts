/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  AcpxRuntime,
  type OpenclawGatewayAccessor,
} from '../../../lib/agents/acpx-runtime'
import {
  type ActiveTurnInfo,
  type TurnFrame,
  TurnRegistry,
} from '../../../lib/agents/active-turn-registry'
import type { AgentDefinition } from '../../../lib/agents/agent-types'
import {
  type CreateAgentInput,
  FileAgentStore,
} from '../../../lib/agents/file-agent-store'
import type {
  AgentHistoryPage,
  AgentRuntime,
  AgentStreamEvent,
} from '../../../lib/agents/types'
import { logger } from '../../../lib/logger'
import type { OpenClawGatewayChatClient } from '../openclaw/openclaw-gateway-chat-client'

export type AgentLiveness = 'working' | 'idle' | 'asleep' | 'error'

export interface AgentActivity {
  status: AgentLiveness
  /** Wall-clock ms; null when the agent has never been used. */
  lastUsedAt: number | null
}

export interface AgentDefinitionWithActivity extends AgentDefinition {
  status: AgentLiveness
  lastUsedAt: number | null
}

/**
 * `idle` downgrades to `asleep` after this many ms of no activity. Read at
 * enrichment time; no timer cleanup necessary.
 */
const ASLEEP_THRESHOLD_MS = 15 * 60 * 1000

/**
 * Provisions and tears down agent records on the OpenClaw gateway side.
 * OpenClaw agents are dual-tracked: the harness owns the user-facing
 * AgentDefinition record while the gateway owns the actual provider
 * config + workspace. Both stores must stay in sync.
 *
 * The interface is decoupled from OpenClawService so the harness can be
 * tested without a live gateway.
 */
export interface OpenClawProvisioner {
  createAgent(input: {
    name: string
    providerType?: string
    providerName?: string
    baseUrl?: string
    apiKey?: string
    modelId?: string
    supportsImages?: boolean
  }): Promise<unknown>
  removeAgent(agentId: string): Promise<void>
  /**
   * Lists agents currently registered on the gateway. Used by the
   * harness reconciliation pass to backfill harness records for
   * gateway-side agents that pre-date the dual-creation flow.
   */
  listAgents(): Promise<
    Array<{ agentId: string; name: string; model?: string }>
  >
  /**
   * Optional. When wired, the harness exposes the gateway lifecycle
   * snapshot through `GET /agents` so the agents page can render
   * Running / Control plane connected pills without a separate
   * `/claw/status` poll. Returns the same shape as the legacy
   * endpoint; `null` when the snapshot can't be fetched (e.g. the
   * gateway is not configured at all).
   */
  getStatus?(): Promise<GatewayStatusSnapshot | null>
}

/**
 * Mirrors the wire shape `/claw/status` returns. Carried through the
 * harness so the agents page has one polling source for everything it
 * renders. Field optionality matches the legacy response.
 */
export interface GatewayStatusSnapshot {
  status: 'uninitialized' | 'starting' | 'running' | 'stopped' | 'error'
  podmanAvailable: boolean
  machineReady: boolean
  port: number | null
  agentCount: number
  error: string | null
  controlPlaneStatus:
    | 'disconnected'
    | 'connecting'
    | 'connected'
    | 'reconnecting'
    | 'recovering'
    | 'failed'
  lastGatewayError: string | null
  lastRecoveryReason:
    | 'transient_disconnect'
    | 'signature_expired'
    | 'pairing_required'
    | 'token_mismatch'
    | 'container_not_ready'
    | 'unknown'
    | null
}

export class AgentHarnessService {
  private readonly agentStore: FileAgentStore
  private readonly runtime: AgentRuntime
  private readonly openclawProvisioner: OpenClawProvisioner | null
  private readonly turnRegistry: TurnRegistry
  private inFlightReconcile: Promise<void> | null = null
  // In-memory liveness tracker. Lost on server restart (acceptable —
  // `lastUsedAt` survives via the acpx session record's `lastUsedAt`,
  // and an idle/asleep agent post-restart will read fine from the
  // record's timestamp without ever flipping to `working`).
  private readonly activity = new Map<
    string,
    { status: 'working' | 'error'; lastEventAt: number; lastError?: string }
  >()

  constructor(
    deps: {
      agentStore?: FileAgentStore
      runtime?: AgentRuntime
      browserosServerPort?: number
      openclawGateway?: OpenclawGatewayAccessor
      openclawGatewayChat?: OpenClawGatewayChatClient
      openclawProvisioner?: OpenClawProvisioner
      turnRegistry?: TurnRegistry
    } = {},
  ) {
    this.agentStore = deps.agentStore ?? new FileAgentStore()
    this.runtime =
      deps.runtime ??
      new AcpxRuntime({
        browserosServerPort: deps.browserosServerPort,
        openclawGateway: deps.openclawGateway,
        openclawGatewayChat: deps.openclawGatewayChat,
      })
    this.openclawProvisioner = deps.openclawProvisioner ?? null
    this.turnRegistry = deps.turnRegistry ?? new TurnRegistry()
  }

  async listAgents(): Promise<AgentDefinition[]> {
    await this.ensureGatewayReconciled()
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
    const lastUsedMap = await this.collectLastUsed(agents)
    const now = Date.now()
    return agents.map((agent) => {
      const live = this.activity.get(agent.id)
      const lastUsedAt = lastUsedMap.get(agent.id) ?? null
      return {
        ...agent,
        status: deriveStatus(live, lastUsedAt, now),
        lastUsedAt,
      }
    })
  }

  /**
   * Read the gateway lifecycle snapshot through the wired provisioner.
   * Returns null if no provisioner is configured or it doesn't expose
   * `getStatus`; route-layer callers should treat that as "no gateway,
   * skip rendering OpenClaw-only chrome." Errors get logged + swallowed
   * so a transient gateway issue doesn't 500 the listing endpoint.
   */
  async getGatewayStatus(): Promise<GatewayStatusSnapshot | null> {
    if (!this.openclawProvisioner?.getStatus) return null
    try {
      return await this.openclawProvisioner.getStatus()
    } catch (err) {
      logger.warn('Failed to fetch gateway status for /agents listing', {
        error: err instanceof Error ? err.message : String(err),
      })
      return null
    }
  }

  /**
   * Read each agent's `lastUsedAt` from the acpx session record (the
   * runtime exposes it through `getHistory` indirectly, but we don't
   * need history items here — only the timestamp). Loads in parallel
   * and tolerates per-agent failures (agents that have never had a
   * turn won't have a record yet).
   */
  private async collectLastUsed(
    agents: AgentDefinition[],
  ): Promise<Map<string, number>> {
    const out = new Map<string, number>()
    await Promise.all(
      agents.map(async (agent) => {
        try {
          const page = await this.runtime.getHistory({
            agent,
            sessionId: 'main',
          })
          const last = page.items.at(-1)?.createdAt
          if (typeof last === 'number' && Number.isFinite(last)) {
            out.set(agent.id, last)
          }
        } catch {
          // No record yet — treat as never-used.
        }
      }),
    )
    return out
  }

  /** Mark `agentId` as actively running a turn. */
  notifyTurnStarted(agentId: string): void {
    this.activity.set(agentId, { status: 'working', lastEventAt: Date.now() })
  }

  /** Clear the working flag. `error` keeps the row badged as needing attention. */
  notifyTurnEnded(
    agentId: string,
    outcome: { ok: boolean; error?: string } = { ok: true },
  ): void {
    if (!outcome.ok) {
      this.activity.set(agentId, {
        status: 'error',
        lastEventAt: Date.now(),
        lastError: outcome.error,
      })
      return
    }
    // Successful turn — drop the in-memory entry. Liveness will be
    // derived from the session record's `lastUsedAt` on next read.
    this.activity.delete(agentId)
  }

  private ensureGatewayReconciled(): Promise<void> {
    // Dedupe concurrent listAgents calls into a single in-flight reconcile,
    // but never memoize the result — agents can be added to the gateway
    // between list calls (e.g. via the legacy /claw/agents create path or
    // out-of-band CLI), and the harness needs to pick those up on the
    // next read. Reconcile is one cheap CLI call and is idempotent.
    if (this.inFlightReconcile) return this.inFlightReconcile
    const run = this.reconcileWithGateway()
      .catch((err) => {
        logger.warn('Harness gateway reconciliation failed', {
          error: err instanceof Error ? err.message : String(err),
        })
      })
      .finally(() => {
        this.inFlightReconcile = null
      })
    this.inFlightReconcile = run
    return run
  }

  async createAgent(input: CreateAgentInput): Promise<AgentDefinition> {
    const agent = await this.agentStore.create(input)

    if (agent.adapter !== 'openclaw') {
      return agent
    }

    if (!this.openclawProvisioner) {
      // Compensating delete keeps the harness store consistent with
      // the failure mode the caller will see (no agent created).
      await this.agentStore.delete(agent.id).catch(() => {})
      throw new OpenClawProvisionerUnavailableError()
    }

    try {
      await this.openclawProvisioner.createAgent({
        name: agent.id,
        providerType: input.providerType,
        providerName: input.providerName,
        baseUrl: input.baseUrl,
        apiKey: input.apiKey,
        modelId: input.modelId,
        supportsImages: input.supportsImages,
      })
      return agent
    } catch (err) {
      logger.warn(
        'OpenClaw gateway provisioning failed; rolling back harness record',
        {
          agentId: agent.id,
          error: err instanceof Error ? err.message : String(err),
        },
      )
      await this.agentStore.delete(agent.id).catch((delErr) => {
        logger.error('Compensating delete failed after provisioning error', {
          agentId: agent.id,
          error: delErr instanceof Error ? delErr.message : String(delErr),
        })
      })
      throw err
    }
  }

  /**
   * Pulls every gateway-side OpenClaw agent into the harness store as a
   * harness record (idempotent, safe to call repeatedly). This lets
   * legacy gateway-only agents — including the always-present `main`
   * sandbox and any orphans from rolled-back dual-creates — surface
   * through the unified `/agents/*` API and route through the harness
   * chat path. After this runs, the rail dedup in the UI keeps a
   * single entry per agent (the harness one wins).
   *
   * Failures are logged and swallowed: the harness must still come up
   * if the gateway is unreachable at boot.
   */
  async reconcileWithGateway(): Promise<void> {
    if (!this.openclawProvisioner) return
    let gatewayAgents: Awaited<ReturnType<OpenClawProvisioner['listAgents']>>
    try {
      gatewayAgents = await this.openclawProvisioner.listAgents()
    } catch (err) {
      logger.warn('Gateway listAgents failed during harness reconciliation', {
        error: err instanceof Error ? err.message : String(err),
      })
      return
    }
    const existing = await this.agentStore.list()
    const existingIds = new Set(existing.map((agent) => agent.id))
    let backfilled = 0
    for (const gatewayAgent of gatewayAgents) {
      if (existingIds.has(gatewayAgent.agentId)) continue
      try {
        await this.agentStore.upsertExisting({
          id: gatewayAgent.agentId,
          name: gatewayAgent.name,
          adapter: 'openclaw',
        })
        backfilled += 1
      } catch (err) {
        logger.warn('Failed to backfill harness record for gateway agent', {
          agentId: gatewayAgent.agentId,
          error: err instanceof Error ? err.message : String(err),
        })
      }
    }
    if (backfilled > 0) {
      logger.info('Harness reconciled with gateway', {
        backfilled,
        gatewayCount: gatewayAgents.length,
      })
    }
  }

  async deleteAgent(agentId: string): Promise<boolean> {
    const agent = await this.agentStore.get(agentId)
    if (!agent) return false

    if (agent.adapter === 'openclaw' && this.openclawProvisioner) {
      try {
        await this.openclawProvisioner.removeAgent(agentId)
      } catch (err) {
        // Tolerate gateway-side removal failure: the harness record is
        // the user-facing identity, so we still want it gone. The orphan
        // gateway agent can be cleaned up out-of-band.
        logger.warn(
          'OpenClaw gateway removeAgent failed; deleting harness record anyway',
          {
            agentId,
            error: err instanceof Error ? err.message : String(err),
          },
        )
      }
    }

    return this.agentStore.delete(agentId)
  }

  getAgent(agentId: string): Promise<AgentDefinition | null> {
    return this.agentStore.get(agentId)
  }

  async getHistory(agentId: string): Promise<AgentHistoryPage> {
    const agent = await this.requireAgent(agentId)
    return this.runtime.getHistory({ agent, sessionId: 'main' })
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
    message: string
    attachments?: ReadonlyArray<{ mediaType: string; data: string }>
  }): Promise<{ turnId: string; frames: ReadableStream<TurnFrame> }> {
    const agent = await this.requireAgent(input.agentId)

    const existing = this.turnRegistry.getActiveFor(agent.id, 'main')
    if (existing) {
      throw new TurnAlreadyActiveError(agent.id, existing.turnId)
    }

    const turn = this.turnRegistry.register(agent.id, 'main')
    this.notifyTurnStarted(agent.id)

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
    sessionId: 'main' = 'main',
  ): ActiveTurnInfo | null {
    const turn = this.turnRegistry.getActiveFor(agentId, sessionId)
    return turn ? this.turnRegistry.describe(turn.turnId) : null
  }

  /**
   * Cancel an active turn. Idempotent — returns true on the first
   * successful cancel, false if the turn doesn't exist or already
   * finished.
   */
  cancelTurn(input: {
    agentId: string
    turnId?: string
    reason?: string
  }): boolean {
    const turnId =
      input.turnId ??
      this.turnRegistry.getActiveFor(input.agentId, 'main')?.turnId
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
    message: string
    attachments?: ReadonlyArray<{ mediaType: string; data: string }>
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
    },
  ): Promise<void> {
    const turn = this.turnRegistry.get(turnId)
    if (!turn) return
    let lastErrorMessage: string | undefined
    try {
      const upstream = await this.runtime.send({
        agent,
        sessionId: 'main',
        sessionKey: agent.sessionKey,
        message: input.message,
        attachments: input.attachments,
        permissionMode: agent.permissionMode,
        signal: turn.abortController.signal,
      })
      const reader = upstream.getReader()
      try {
        while (true) {
          const { done, value } = await reader.read()
          if (done) break
          if (value.type === 'error') lastErrorMessage = value.message
          this.turnRegistry.pushEvent(turnId, value)
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
      this.notifyTurnEnded(agent.id, {
        ok: lastErrorMessage === undefined,
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
 * Thrown when an `openclaw` adapter agent is created on a harness that
 * has no OpenClaw provisioner wired in. Surfaces as a 503 in the route
 * layer so callers know the service is misconfigured rather than a
 * client-side input error.
 */
export class OpenClawProvisionerUnavailableError extends Error {
  constructor() {
    super('OpenClaw gateway provisioner is not wired into AgentHarnessService')
    this.name = 'OpenClawProvisionerUnavailableError'
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
