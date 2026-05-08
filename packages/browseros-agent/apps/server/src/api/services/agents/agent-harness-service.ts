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
import type {
  AgentStore,
  CreateAgentInput,
} from '../../../lib/agents/agent-store'
import type { AgentDefinition } from '../../../lib/agents/agent-types'
import { DbAgentStore } from '../../../lib/agents/db-agent-store'
import {
  FileMessageQueue,
  type QueuedMessage,
  type QueuedMessageAttachment,
} from '../../../lib/agents/message-queue'
import { writeHermesPerAgentProvider } from '../hermes/hermes-paths'
import { getHermesProviderMapping } from '../hermes/hermes-provider-map'

export {
  MessageQueueFullError,
  type QueuedMessage,
  type QueuedMessageAttachment,
} from '../../../lib/agents/message-queue'

import { basename } from 'node:path'
import type {
  AgentHistoryPage,
  AgentRowSnapshot,
  AgentRuntime,
  AgentStreamEvent,
} from '../../../lib/agents/types'
import { getOpenClawDir } from '../../../lib/browseros-dir'
import { logger } from '../../../lib/logger'
import {
  buildFilePreview,
  detectMimeType,
  type FilePreview,
} from '../openclaw/file-preview'
import { getHostWorkspaceDir } from '../openclaw/openclaw-env'
import {
  type FileSnapshot,
  type ProducedFileRow,
  ProducedFilesStore,
} from '../openclaw/produced-files-store'

export type AgentLiveness = 'working' | 'idle' | 'asleep' | 'error'

export interface AgentActivity {
  status: AgentLiveness
  /** Wall-clock ms; null when the agent has never been used. */
  lastUsedAt: number | null
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
  /**
   * Optional. When wired, the harness uses this for `getHistory` on
   * openclaw-adapter agents so the chat panel sees autonomous
   * (cron / hook / channel) turns alongside user-typed turns. Without
   * this, history reads come from AcpxRuntime's local session record
   * which only contains user-initiated turns — autonomous activity
   * fires correctly but stays invisible to the panel.
   */
  getAgentHistory?(agentId: string): Promise<AgentHistoryPage>
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

/**
 * Per-turn event the harness emits to subscribers. Lets services that
 * want to track liveness for a specific adapter (e.g. OpenClaw's
 * ClawSession dashboard) react to the same stream the chat panel sees,
 * without each adapter spawning its own gateway-side observer.
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
  private readonly openclawProvisioner: OpenClawProvisioner | null
  private readonly turnRegistry: TurnRegistry
  private readonly messageQueue: FileMessageQueue
  private readonly turnLifecycleListeners = new Set<TurnLifecycleListener>()
  /**
   * Optional override for the BrowserOS dir used by Hermes per-agent
   * provider config writes. Defaults to the global `getBrowserosDir()`
   * lookup at write time when undefined; tests can inject a tmp dir.
   */
  private readonly browserosDir: string | undefined
  /**
   * Lazy-initialised so tests that swap in a fake `agentStore` don't
   * eagerly hit `getDb()` (which throws when the test harness hasn't
   * called `initializeDb`). Tests that exercise file attribution can
   * inject an explicit store via `deps.producedFilesStore`.
   */
  private explicitProducedFilesStore: ProducedFilesStore | null = null
  private cachedProducedFilesStore: ProducedFilesStore | null = null
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
      agentStore?: AgentStore
      runtime?: AgentRuntime
      browserosServerPort?: number
      browserosDir?: string
      openclawGateway?: OpenclawGatewayAccessor
      openclawProvisioner?: OpenClawProvisioner
      turnRegistry?: TurnRegistry
      messageQueue?: FileMessageQueue
      producedFilesStore?: ProducedFilesStore
    } = {},
  ) {
    this.agentStore = deps.agentStore ?? new DbAgentStore()
    this.runtime =
      deps.runtime ??
      new AcpxRuntime({
        browserosServerPort: deps.browserosServerPort,
        openclawGateway: deps.openclawGateway,
      })
    this.openclawProvisioner = deps.openclawProvisioner ?? null
    this.turnRegistry = deps.turnRegistry ?? new TurnRegistry()
    this.messageQueue = deps.messageQueue ?? new FileMessageQueue()
    this.browserosDir = deps.browserosDir
    if (deps.producedFilesStore) {
      this.explicitProducedFilesStore = deps.producedFilesStore
    }
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
    const [snapshots, queueSnapshot] = await Promise.all([
      this.collectRowSnapshots(agents),
      this.messageQueue.snapshotAll(),
    ])
    const now = Date.now()
    return agents.map((agent) => {
      const live = this.activity.get(agent.id)
      const snapshot = snapshots.get(agent.id) ?? null
      const lastUsedAt = snapshot?.lastUsedAt ?? null
      const activeTurn = this.turnRegistry.getActiveFor(agent.id, 'main')
      return {
        ...agent,
        pinned: agent.pinned ?? false,
        status: deriveStatus(live, lastUsedAt, now),
        lastUsedAt,
        lastUserMessage: snapshot?.lastUserMessage ?? null,
        cwd: snapshot?.cwd ?? null,
        tokens: snapshot?.tokens ?? null,
        turnsByDay: ZERO_BUCKETS(),
        failedByDay: ZERO_BUCKETS(),
        lastError: live?.status === 'error' ? (live.lastError ?? null) : null,
        lastErrorAt:
          live?.status === 'error' ? (live.lastEventAt ?? null) : null,
        activeTurnId: activeTurn?.turnId ?? null,
        queue: queueSnapshot[agent.id] ?? [],
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
    if (typeof this.runtime.getRowSnapshot === 'function') {
      return this.runtime.getRowSnapshot({ agent, sessionId: 'main' })
    }
    // Legacy fallback: derive only `lastUsedAt` from the history page.
    const page = await this.runtime.getHistory({ agent, sessionId: 'main' })
    const last = page.items.at(-1)?.createdAt
    if (typeof last !== 'number' || !Number.isFinite(last)) return null
    return {
      cwd: null,
      lastUsedAt: last,
      lastUserMessage: null,
      tokens: null,
    }
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
    } else {
      // Successful turn — drop the in-memory entry. Liveness will be
      // derived from the session record's `lastUsedAt` on next read.
      this.activity.delete(agentId)
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
    // Race guard: a turn may have started between `popOldest` and now
    // (e.g. the user typed and clicked Send directly between cancel
    // and the drain). Put the message back at the head and let the
    // next turn-end retry.
    if (this.turnRegistry.getActiveFor(agentId, 'main')) {
      await this.messageQueue.pushFront(agentId, next)
      return
    }
    try {
      await this.startTurn({
        agentId,
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
    message: string
    attachments?: ReadonlyArray<QueuedMessageAttachment>
  }): Promise<QueuedMessage> {
    const agent = await this.requireAgent(input.agentId)
    const queued = await this.messageQueue.append(agent.id, {
      message: input.message,
      attachments: input.attachments,
    })
    // Defensive drain: if the agent has no active turn at enqueue
    // time (e.g. the user enqueued during the brief window between
    // turns), pop it back off and start it directly. Avoids the
    // queue sitting idle while the agent is also idle.
    if (!this.turnRegistry.getActiveFor(agent.id, 'main')) {
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
    if (input.adapter === 'hermes') {
      // Validate before touching the store so we don't leave an orphan
      // record on the unhappy path.
      assertHermesProviderInputValid(input)
    }

    const agent = await this.agentStore.create(input)

    if (agent.adapter === 'hermes') {
      try {
        await this.writeHermesPerAgentProvider(agent.id, input)
      } catch (err) {
        await this.agentStore.delete(agent.id).catch(() => {})
        throw err
      }
      return agent
    }

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
   * Write Hermes' per-agent config.yaml + .env into the on-host home
   * dir. Caller must have already run assertHermesProviderInputValid;
   * any throw here is a real I/O failure and must roll back the agent
   * record.
   */
  private async writeHermesPerAgentProvider(
    agentId: string,
    input: CreateAgentInput,
  ): Promise<void> {
    // Non-null assertions are safe: assertHermesProviderInputValid ran
    // first and rejects when any required field is missing.
    const mapping = getHermesProviderMapping(input.providerType as string)
    if (!mapping) {
      throw new HermesProviderConfigInvalidError(
        `Provider type "${input.providerType}" is not supported by Hermes`,
      )
    }
    await writeHermesPerAgentProvider({
      browserosDir: this.browserosDir,
      agentId,
      providerId: mapping.hermesProvider,
      envVarName: mapping.envVarName,
      apiKey: (input.apiKey as string).trim(),
      modelId: (input.modelId as string).trim(),
      baseUrl: input.baseUrl?.trim() || mapping.defaultBaseUrl,
    })
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

  async getHistory(agentId: string): Promise<AgentHistoryPage> {
    const agent = await this.requireAgent(agentId)
    // OpenClaw agents persist conversation in the gateway, not in the
    // AcpxRuntime's local session record. Reading the local record
    // would miss autonomous (cron / hook / channel) turns. Route
    // through the provisioner so the panel sees the full history.
    if (
      agent.adapter === 'openclaw' &&
      this.openclawProvisioner?.getAgentHistory
    ) {
      return this.openclawProvisioner.getAgentHistory(agentId)
    }
    return this.runtime.getHistory({ agent, sessionId: 'main' })
  }

  // ── Produced files (Files rail / inline artifact card) ───────────

  /**
   * Outputs-rail data for one agent. Returns groups of files keyed
   * by the assistant turn that produced them, newest first. Empty
   * array when the agent hasn't produced anything yet, or when the
   * adapter doesn't track outputs (claude / codex — see Phase 2
   * commit).
   */
  async listAgentFiles(
    agentId: string,
    options: { limit?: number } = {},
  ): Promise<ProducedFilesRailGroup[]> {
    const agent = await this.requireAgent(agentId)
    const store = this.tryGetProducedFilesStore()
    if (!store) return []
    const rows = await store.listByAgent(agent.id, options)
    return store
      .groupByTurn(rows)
      .map(({ turnId, turnPrompt, createdAt, files }) => ({
        turnId,
        turnPrompt,
        createdAt,
        files: files.map(toProducedFileEntry),
      }))
  }

  /**
   * Inline-card data for one assistant turn. Used by the SSE
   * `produced_files` event consumer to refresh metadata after the
   * turn completes; also handy for direct fetches by clients that
   * missed the live event.
   */
  async listAgentFilesForTurn(
    agentId: string,
    turnId: string,
  ): Promise<ProducedFileEntry[]> {
    await this.requireAgent(agentId)
    const store = this.tryGetProducedFilesStore()
    if (!store) return []
    const rows = await store.listByTurn(turnId)
    return rows.map(toProducedFileEntry)
  }

  /**
   * Build a preview payload for a single file. Returns null when the
   * file id is unknown OR the on-disk path no longer exists. The
   * route layer maps null → 404.
   */
  async previewProducedFile(fileId: string): Promise<FilePreview | null> {
    const store = this.tryGetProducedFilesStore()
    if (!store) return null
    const row = await store.findById(fileId)
    if (!row) return null
    const agent = await this.agentStore.get(row.agentDefinitionId)
    if (!agent || agent.adapter !== 'openclaw') return null
    const workspaceDir = getHostWorkspaceDir(getOpenClawDir(), agent.name)
    const resolved = await store.resolveFilePath({ fileId, workspaceDir })
    if (!resolved) return null
    return buildFilePreview(resolved.absolutePath)
  }

  /**
   * Resolve a file id to an absolute on-disk path + metadata for the
   * download route to stream. Null when the file id is unknown or
   * the path escaped the workspace root (containment check happens
   * inside `producedFilesStore.resolveFilePath`).
   */
  async resolveProducedFileForDownload(fileId: string): Promise<{
    absolutePath: string
    fileName: string
    mimeType: string
    size: number
  } | null> {
    const store = this.tryGetProducedFilesStore()
    if (!store) return null
    const row = await store.findById(fileId)
    if (!row) return null
    const agent = await this.agentStore.get(row.agentDefinitionId)
    if (!agent || agent.adapter !== 'openclaw') return null
    const workspaceDir = getHostWorkspaceDir(getOpenClawDir(), agent.name)
    const resolved = await store.resolveFilePath({ fileId, workspaceDir })
    if (!resolved) return null
    const mimeType = await detectMimeType(resolved.absolutePath)
    const fileName = basename(row.path)
    return {
      absolutePath: resolved.absolutePath,
      fileName,
      mimeType,
      size: row.size,
    }
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
    cwd?: string
  }): Promise<{ turnId: string; frames: ReadableStream<TurnFrame> }> {
    const agent = await this.requireAgent(input.agentId)

    const existing = this.turnRegistry.getActiveFor(agent.id, 'main')
    if (existing) {
      throw new TurnAlreadyActiveError(agent.id, existing.turnId)
    }

    const turn = this.turnRegistry.register(agent.id, 'main', {
      prompt: input.message,
    })
    this.notifyTurnStarted(agent.id)
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
    let lastErrorMessage: string | undefined

    // Bracket openclaw turns with a workspace snapshot so any file the
    // agent produces during the turn is attributable back to it (rail
    // + inline artifact UX). Adapter-gated for v1 — Claude / Codex
    // write to the user's host filesystem and don't need this; their
    // outputs are already visible via the user's own tools.
    const isOpenclaw = agent.adapter === 'openclaw'
    const workspaceDir = isOpenclaw ? this.resolveSafeWorkspaceDir(agent) : null
    const producedFilesStore = workspaceDir
      ? this.tryGetProducedFilesStore()
      : null
    const workspaceSnapshot =
      workspaceDir && producedFilesStore
        ? await this.snapshotWorkspaceForTurn(
            agent,
            workspaceDir,
            producedFilesStore,
          )
        : null

    try {
      const upstream = await this.runtime.send({
        agent,
        sessionId: 'main',
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
      // Attribute any files the agent produced during this turn. We
      // run on success, error, AND inside `finally` so an upstream
      // failure mid-turn that still managed to write files doesn't
      // lose them. We skip only when the user explicitly cancelled —
      // in that case the side effects shouldn't be surfaced as
      // "outputs you asked for."
      if (
        workspaceDir &&
        workspaceSnapshot !== null &&
        producedFilesStore &&
        !turn.abortController.signal.aborted
      ) {
        await this.attributeTurnFiles({
          producedFilesStore,
          workspaceDir,
          before: workspaceSnapshot,
          agent,
          turnId,
          turnPrompt: input.message,
        })
      }
      this.notifyTurnEnded(agent.id, {
        ok: lastErrorMessage === undefined,
        error: lastErrorMessage,
      })
      this.emitTurnLifecycle(agent, {
        type: 'turn_ended',
        error: lastErrorMessage,
      })
    }
  }

  /**
   * Compute the host-side workspace dir for an openclaw agent,
   * returning `null` when the agent's display name fails the
   * path-traversal guard. Logs a warning so the safety-disabled
   * case is observable in production.
   */
  private resolveSafeWorkspaceDir(agent: AgentDefinition): string | null {
    try {
      return getHostWorkspaceDir(getOpenClawDir(), agent.name)
    } catch (err) {
      logger.warn('Skipping openclaw file attribution: unsafe agent name', {
        agentId: agent.id,
        agentName: agent.name,
        error: err instanceof Error ? err.message : String(err),
      })
      return null
    }
  }

  /**
   * Pre-turn workspace snapshot. Returns `null` on any failure so
   * the rest of the turn flow continues without file attribution.
   */
  private async snapshotWorkspaceForTurn(
    agent: AgentDefinition,
    workspaceDir: string,
    producedFilesStore: ProducedFilesStore,
  ): Promise<FileSnapshot | null> {
    try {
      return await producedFilesStore.snapshotWorkspace(workspaceDir)
    } catch (err) {
      logger.warn(
        'Failed to snapshot openclaw workspace; file attribution disabled for this turn',
        {
          agentId: agent.id,
          workspaceDir,
          error: err instanceof Error ? err.message : String(err),
        },
      )
      return null
    }
  }

  /**
   * Lazily resolve the produced-files store. Returns `null` if the
   * SQLite handle isn't initialised yet — keeps the harness usable in
   * tests + during early server boot, where chat turns are unlikely
   * but allowed.
   */
  private tryGetProducedFilesStore(): ProducedFilesStore | null {
    if (this.explicitProducedFilesStore) return this.explicitProducedFilesStore
    if (this.cachedProducedFilesStore) return this.cachedProducedFilesStore
    try {
      this.cachedProducedFilesStore = new ProducedFilesStore()
      return this.cachedProducedFilesStore
    } catch (err) {
      logger.warn(
        'Produced-files store unavailable; turn-level file attribution disabled',
        { error: err instanceof Error ? err.message : String(err) },
      )
      return null
    }
  }

  /**
   * Diff the workspace, persist new/modified files, and emit a
   * `produced_files` event so subscribers can render the inline
   * artifact card. Tolerant of all errors — a failure here must
   * never block the rest of the turn-end bookkeeping.
   */
  private async attributeTurnFiles(input: {
    producedFilesStore: ProducedFilesStore
    workspaceDir: string
    before: FileSnapshot
    agent: AgentDefinition
    turnId: string
    turnPrompt: string
  }): Promise<void> {
    try {
      const rows = await input.producedFilesStore.finalizeTurn({
        agentDefinitionId: input.agent.id,
        sessionKey: input.agent.sessionKey,
        turnId: input.turnId,
        turnPrompt: input.turnPrompt,
        workspaceDir: input.workspaceDir,
        before: input.before,
      })
      if (rows.length === 0) return
      this.turnRegistry.pushEvent(input.turnId, {
        type: 'produced_files',
        files: rows.map((row) => ({
          id: row.id,
          path: row.path,
          size: row.size,
          mtimeMs: row.mtimeMs,
        })),
      })
    } catch (err) {
      logger.warn('Failed to attribute produced files for turn', {
        agentId: input.agent.id,
        turnId: input.turnId,
        error: err instanceof Error ? err.message : String(err),
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
 * Thrown when a Hermes adapter agent is created without a complete
 * provider config (provider type, API key, model id; base URL when the
 * provider mapping requires it). Surfaces as a 400 in the route layer.
 */
export class HermesProviderConfigInvalidError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'HermesProviderConfigInvalidError'
  }
}

function assertHermesProviderInputValid(input: CreateAgentInput): void {
  const providerType = input.providerType?.trim()
  if (!providerType) {
    throw new HermesProviderConfigInvalidError(
      'Hermes agent requires providerType (pick a provider configured in BrowserOS AI Settings)',
    )
  }
  const mapping = getHermesProviderMapping(providerType)
  if (!mapping) {
    throw new HermesProviderConfigInvalidError(
      `Provider type "${providerType}" is not supported by Hermes`,
    )
  }
  if (!input.apiKey?.trim()) {
    throw new HermesProviderConfigInvalidError(
      'Hermes agent requires apiKey from the selected provider',
    )
  }
  if (!input.modelId?.trim()) {
    throw new HermesProviderConfigInvalidError(
      'Hermes agent requires modelId from the selected provider',
    )
  }
  if (mapping.requiresBaseUrl && !input.baseUrl?.trim()) {
    throw new HermesProviderConfigInvalidError(
      `Provider type "${providerType}" requires baseUrl`,
    )
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

// ── Files API DTO ────────────────────────────────────────────────

/**
 * Wire shape for one produced-file entry returned by the rail and
 * inline-card endpoints. Trimmed from the on-disk row — clients
 * never see `agentDefinitionId` or `sessionKey`.
 */
export interface ProducedFileEntry {
  id: string
  path: string
  size: number
  mtimeMs: number
  createdAt: number
  detectedBy: 'diff' | 'tool'
}

export interface ProducedFilesRailGroup {
  turnId: string
  /** First non-blank line of the user prompt that initiated this turn. */
  turnPrompt: string
  createdAt: number
  files: ProducedFileEntry[]
}

function toProducedFileEntry(row: ProducedFileRow): ProducedFileEntry {
  return {
    id: row.id,
    path: row.path,
    size: row.size,
    mtimeMs: row.mtimeMs,
    createdAt: row.createdAt,
    detectedBy: row.detectedBy,
  }
}
