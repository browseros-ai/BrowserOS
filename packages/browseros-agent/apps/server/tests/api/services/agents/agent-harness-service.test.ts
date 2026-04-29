/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { AgentHarnessService } from '../../../../src/api/services/agents/agent-harness-service'
import type { AgentDefinition } from '../../../../src/lib/agents/agent-types'
import type { FileAgentStore } from '../../../../src/lib/agents/file-agent-store'
import type {
  AgentRuntime,
  AgentStreamEvent,
} from '../../../../src/lib/agents/types'

describe('AgentHarnessService', () => {
  it('creates named agents and sends prompts through the main session', async () => {
    const agents: AgentDefinition[] = []
    const runtimeInputs: unknown[] = []
    const agentStore = createAgentStore(agents)
    const runtime: AgentRuntime = {
      async status() {
        return { state: 'ready' }
      },
      async listSessions() {
        return []
      },
      async getHistory() {
        return { agentId: 'agent-1', sessionId: 'main', items: [] }
      },
      async send(input) {
        runtimeInputs.push(input)
        return new ReadableStream<AgentStreamEvent>({
          start(controller) {
            controller.enqueue({
              type: 'text_delta',
              text: 'answer',
              stream: 'output',
            })
            controller.enqueue({ type: 'done', stopReason: 'end_turn' })
            controller.close()
          },
        })
      },
    }

    const service = new AgentHarnessService({
      agentStore: agentStore as FileAgentStore,
      runtime,
    })

    const agent = await service.createAgent({
      name: 'Review bot',
      adapter: 'codex',
      modelId: 'gpt-5.5',
      reasoningEffort: 'medium',
    })
    const events = await collectStream(
      await service.send({
        agentId: agent.id,
        message: 'hello',
      }),
    )

    expect(runtimeInputs[0]).toMatchObject({
      agent,
      sessionId: 'main',
      sessionKey: 'agent:agent-1:main',
      message: 'hello',
      permissionMode: 'approve-all',
    })
    expect(events).toEqual([
      { type: 'text_delta', text: 'answer', stream: 'output' },
      { type: 'done', stopReason: 'end_turn' },
    ])
  })

  it('reads history from the runtime', async () => {
    const agent: AgentDefinition = {
      id: 'agent-1',
      name: 'Review bot',
      adapter: 'codex',
      modelId: 'gpt-5.5',
      reasoningEffort: 'medium',
      permissionMode: 'approve-all',
      sessionKey: 'agent:agent-1:main',
      createdAt: 1000,
      updatedAt: 1000,
    }
    const runtimeInputs: unknown[] = []
    const runtime: AgentRuntime = {
      async status() {
        return { state: 'ready' }
      },
      async listSessions() {
        return []
      },
      async getHistory(input) {
        runtimeInputs.push(input)
        return {
          agentId: agent.id,
          sessionId: 'main',
          items: [
            {
              id: 'agent:agent-1:main:1',
              agentId: agent.id,
              sessionId: 'main',
              role: 'assistant',
              text: 'Done.',
              createdAt: 1000,
              reasoning: { text: 'checking state' },
              toolCalls: [
                {
                  toolCallId: 'tool-1',
                  toolName: 'read_file',
                  status: 'completed',
                  input: { path: 'src/index.ts' },
                  output: 'file contents',
                },
              ],
            },
          ],
        }
      },
      async send() {
        return new ReadableStream<AgentStreamEvent>()
      },
    }
    const service = new AgentHarnessService({
      agentStore: createAgentStore([agent]) as FileAgentStore,
      runtime,
    })

    const history = await service.getHistory(agent.id)

    expect(runtimeInputs).toEqual([{ agent, sessionId: 'main' }])
    expect(history.items[0]).toMatchObject({
      role: 'assistant',
      reasoning: { text: 'checking state' },
      toolCalls: [{ toolName: 'read_file' }],
    })
  })

  it('dual-creates an OpenClaw adapter agent on the gateway with the harness id as the gateway name', async () => {
    const agents: AgentDefinition[] = []
    const provisionerCalls: Array<{ method: string; input: unknown }> = []
    const provisioner = {
      async createAgent(input: unknown) {
        provisionerCalls.push({ method: 'createAgent', input })
        return { agentId: 'mock', name: 'mock', workspace: '/workspace' }
      },
      async removeAgent(agentId: string) {
        provisionerCalls.push({ method: 'removeAgent', input: agentId })
      },
      async listAgents() {
        return []
      },
    }
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as FileAgentStore,
      runtime: stubRuntime(),
      openclawProvisioner: provisioner,
    })

    const agent = await service.createAgent({
      name: 'OpenClaw bot',
      adapter: 'openclaw',
      providerType: 'openai-compatible',
      providerName: 'Kimi',
      baseUrl: 'https://api.fireworks.ai/inference/v1',
      apiKey: 'test-key',
      modelId: 'accounts/fireworks/models/kimi-k2p5',
      supportsImages: true,
    })

    expect(agent.adapter).toBe('openclaw')
    expect(provisionerCalls).toEqual([
      {
        method: 'createAgent',
        input: {
          name: agent.id,
          providerType: 'openai-compatible',
          providerName: 'Kimi',
          baseUrl: 'https://api.fireworks.ai/inference/v1',
          apiKey: 'test-key',
          modelId: 'accounts/fireworks/models/kimi-k2p5',
          supportsImages: true,
        },
      },
    ])
    expect(agents).toHaveLength(1)
  })

  it('rolls back the harness record when gateway provisioning fails', async () => {
    const agents: AgentDefinition[] = []
    const provisioner = {
      async createAgent() {
        throw new Error('gateway boom')
      },
      async removeAgent() {
        // no-op
      },
      async listAgents() {
        return []
      },
    }
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as FileAgentStore,
      runtime: stubRuntime(),
      openclawProvisioner: provisioner,
    })

    await expect(
      service.createAgent({ name: 'Doomed', adapter: 'openclaw' }),
    ).rejects.toThrow('gateway boom')
    expect(agents).toHaveLength(0)
  })

  it('refuses to create an OpenClaw agent when no provisioner is wired', async () => {
    const agents: AgentDefinition[] = []
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as FileAgentStore,
      runtime: stubRuntime(),
    })

    await expect(
      service.createAgent({ name: 'Stranded', adapter: 'openclaw' }),
    ).rejects.toThrow('OpenClaw gateway provisioner is not wired')
    expect(agents).toHaveLength(0)
  })

  it('removes the gateway agent on delete and tolerates gateway-side failure', async () => {
    const agents: AgentDefinition[] = []
    const provisionerCalls: string[] = []
    let shouldFail = false
    const provisioner = {
      async createAgent() {
        return { agentId: 'mock', name: 'mock', workspace: '/workspace' }
      },
      async removeAgent(agentId: string) {
        provisionerCalls.push(agentId)
        if (shouldFail) throw new Error('gateway down')
      },
      async listAgents() {
        return []
      },
    }
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as FileAgentStore,
      runtime: stubRuntime(),
      openclawProvisioner: provisioner,
    })

    const agent = await service.createAgent({
      name: 'OpenClaw bot',
      adapter: 'openclaw',
    })

    // Happy path: gateway delete succeeds → harness record gone.
    expect(await service.deleteAgent(agent.id)).toBe(true)
    expect(provisionerCalls).toEqual([agent.id])
    expect(agents).toHaveLength(0)

    // Failure path: gateway delete throws → harness record still removed.
    const second = await service.createAgent({
      name: 'OpenClaw bot 2',
      adapter: 'openclaw',
    })
    shouldFail = true
    expect(await service.deleteAgent(second.id)).toBe(true)
    expect(agents).toHaveLength(0)
  })

  it('backfills harness records for gateway agents on first listAgents call', async () => {
    const agents: AgentDefinition[] = []
    const provisioner = {
      async createAgent() {
        return { agentId: 'mock', name: 'mock', workspace: '/workspace' }
      },
      async removeAgent() {
        // no-op
      },
      async listAgents() {
        return [
          { agentId: 'main', name: 'main' },
          { agentId: 'orphan', name: 'orphan' },
        ]
      },
    }
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as FileAgentStore,
      runtime: stubRuntime(),
      openclawProvisioner: provisioner,
    })

    const listed = await service.listAgents()
    expect(listed.map((a) => a.id).sort()).toEqual(['main', 'orphan'])
    expect(listed.every((a) => a.adapter === 'openclaw')).toBe(true)

    // Idempotent: a second listAgents must not duplicate the records.
    const second = await service.listAgents()
    expect(second).toHaveLength(2)
  })

  it('keeps harness usable when gateway listAgents fails during reconciliation', async () => {
    const agents: AgentDefinition[] = [
      {
        id: 'agent-existing',
        name: 'existing',
        adapter: 'claude',
        modelId: 'haiku',
        reasoningEffort: 'medium',
        permissionMode: 'approve-all',
        sessionKey: 'agent:agent-existing:main',
        createdAt: 1000,
        updatedAt: 1000,
      },
    ]
    const provisioner = {
      async createAgent() {
        return { agentId: 'mock', name: 'mock', workspace: '/workspace' }
      },
      async removeAgent() {
        // no-op
      },
      async listAgents() {
        throw new Error('gateway down at boot')
      },
    }
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as FileAgentStore,
      runtime: stubRuntime(),
      openclawProvisioner: provisioner,
    })

    const listed = await service.listAgents()
    expect(listed).toHaveLength(1)
    expect(listed[0]?.id).toBe('agent-existing')
  })
})

function stubRuntime(): AgentRuntime {
  return {
    async status() {
      return { state: 'ready' }
    },
    async listSessions() {
      return []
    },
    async getHistory(input) {
      return { agentId: input.agent.id, sessionId: 'main', items: [] }
    },
    async send() {
      return new ReadableStream<AgentStreamEvent>()
    },
  }
}

function createAgentStore(agents: AgentDefinition[]) {
  return {
    async list() {
      return agents
    },
    async get(id: string) {
      return agents.find((agent) => agent.id === id) ?? null
    },
    async create(input) {
      const agent: AgentDefinition = {
        id: `agent-${agents.length + 1}`,
        name: input.name,
        adapter: input.adapter,
        modelId: input.modelId,
        reasoningEffort: input.reasoningEffort,
        permissionMode: 'approve-all',
        sessionKey: `agent:agent-${agents.length + 1}:main`,
        createdAt: 1000,
        updatedAt: 1000,
      }
      agents.push(agent)
      return agent
    },
    async delete(id: string) {
      const idx = agents.findIndex((agent) => agent.id === id)
      if (idx === -1) return false
      agents.splice(idx, 1)
      return true
    },
    async upsertExisting(input: {
      id: string
      name: string
      adapter: AgentDefinition['adapter']
      modelId?: string
      reasoningEffort?: string
    }) {
      const existing = agents.find((entry) => entry.id === input.id)
      if (existing) return existing
      const agent: AgentDefinition = {
        id: input.id,
        name: input.name,
        adapter: input.adapter,
        modelId: input.modelId ?? 'default',
        reasoningEffort: input.reasoningEffort ?? 'medium',
        permissionMode: 'approve-all',
        sessionKey: `agent:${input.id}:main`,
        createdAt: 1000,
        updatedAt: 1000,
      }
      agents.push(agent)
      return agent
    },
  } satisfies Partial<FileAgentStore>
}

async function collectStream(
  stream: ReadableStream<AgentStreamEvent>,
): Promise<AgentStreamEvent[]> {
  const reader = stream.getReader()
  const events: AgentStreamEvent[] = []
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      events.push(value)
    }
  } finally {
    reader.releaseLock()
  }
  return events
}
