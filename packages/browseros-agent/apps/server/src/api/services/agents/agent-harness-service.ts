/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { AcpxRuntime } from '../../../lib/agents/acpx-runtime'
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

export class AgentHarnessService {
  private readonly agentStore: FileAgentStore
  private readonly runtime: AgentRuntime

  constructor(
    deps: {
      agentStore?: FileAgentStore
      runtime?: AgentRuntime
      browserosServerPort?: number
    } = {},
  ) {
    this.agentStore = deps.agentStore ?? new FileAgentStore()
    this.runtime =
      deps.runtime ??
      new AcpxRuntime({ browserosServerPort: deps.browserosServerPort })
  }

  listAgents(): Promise<AgentDefinition[]> {
    return this.agentStore.list()
  }

  createAgent(input: CreateAgentInput): Promise<AgentDefinition> {
    return this.agentStore.create(input)
  }

  deleteAgent(agentId: string): Promise<boolean> {
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
    signal?: AbortSignal
  }): Promise<ReadableStream<AgentStreamEvent>> {
    const agent = await this.requireAgent(input.agentId)
    return this.runtime.send({
      agent,
      sessionId: 'main',
      sessionKey: agent.sessionKey,
      message: input.message,
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
