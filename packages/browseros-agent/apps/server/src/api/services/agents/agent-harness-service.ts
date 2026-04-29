/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  AcpxRuntime,
  type OpenclawGatewayAccessor,
} from '../../../lib/agents/acpx-runtime'
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
}

export class AgentHarnessService {
  private readonly agentStore: FileAgentStore
  private readonly runtime: AgentRuntime
  private readonly openclawProvisioner: OpenClawProvisioner | null
  private inFlightReconcile: Promise<void> | null = null

  constructor(
    deps: {
      agentStore?: FileAgentStore
      runtime?: AgentRuntime
      browserosServerPort?: number
      openclawGateway?: OpenclawGatewayAccessor
      openclawGatewayChat?: OpenClawGatewayChatClient
      openclawProvisioner?: OpenClawProvisioner
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
  }

  async listAgents(): Promise<AgentDefinition[]> {
    await this.ensureGatewayReconciled()
    return this.agentStore.list()
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

  async send(input: {
    agentId: string
    message: string
    attachments?: ReadonlyArray<{ mediaType: string; data: string }>
    signal?: AbortSignal
  }): Promise<ReadableStream<AgentStreamEvent>> {
    const agent = await this.requireAgent(input.agentId)
    return this.runtime.send({
      agent,
      sessionId: 'main',
      sessionKey: agent.sessionKey,
      message: input.message,
      attachments: input.attachments,
      permissionMode: agent.permissionMode,
      signal: input.signal,
    })
  }

  private async requireAgent(agentId: string): Promise<AgentDefinition> {
    const agent = await this.agentStore.get(agentId)
    if (!agent) {
      throw new UnknownAgentError(agentId)
    }
    return agent
  }
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
