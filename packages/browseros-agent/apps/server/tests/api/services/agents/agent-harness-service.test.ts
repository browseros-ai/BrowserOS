/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { mkdtempSync, readFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { AgentHarnessService } from '../../../../src/api/services/agents/agent-harness-service'
import type { AgentStore } from '../../../../src/lib/agents/agent-store'
import type { AgentDefinition } from '../../../../src/lib/agents/agent-types'
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
      agentStore: agentStore as AgentStore,
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
        cwd: '/tmp/work',
      }),
    )

    expect(runtimeInputs[0]).toMatchObject({
      agent,
      sessionId: 'main',
      sessionKey: 'agent:agent-1:main',
      message: 'hello',
      permissionMode: 'approve-all',
      cwd: '/tmp/work',
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
      agentStore: createAgentStore([agent]) as AgentStore,
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
      agentStore: createAgentStore(agents) as AgentStore,
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
      agentStore: createAgentStore(agents) as AgentStore,
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
      agentStore: createAgentStore(agents) as AgentStore,
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
      agentStore: createAgentStore(agents) as AgentStore,
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
      agentStore: createAgentStore(agents) as AgentStore,
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
      agentStore: createAgentStore(agents) as AgentStore,
      runtime: stubRuntime(),
      openclawProvisioner: provisioner,
    })

    const listed = await service.listAgents()
    expect(listed).toHaveLength(1)
    expect(listed[0]?.id).toBe('agent-existing')
  })

  it('marks an agent working while a turn streams and idle once it ends', async () => {
    const agent: AgentDefinition = {
      id: 'live-1',
      name: 'live',
      adapter: 'claude',
      modelId: 'haiku',
      reasoningEffort: 'medium',
      permissionMode: 'approve-all',
      sessionKey: 'agent:live-1:main',
      createdAt: 1000,
      updatedAt: 1000,
    }
    // Hold the upstream open until the test releases it so we can
    // observe the "working" state between dispatch and stream end.
    let releaseUpstream: () => void = () => {}
    const upstreamHeld = new Promise<void>((resolve) => {
      releaseUpstream = resolve
    })
    const runtime: AgentRuntime = {
      async status() {
        return { state: 'ready' }
      },
      async listSessions() {
        return []
      },
      async getHistory() {
        return { agentId: agent.id, sessionId: 'main', items: [] }
      },
      async send() {
        return new ReadableStream<AgentStreamEvent>({
          async start(controller) {
            controller.enqueue({
              type: 'text_delta',
              text: 'hi',
              stream: 'output',
            })
            await upstreamHeld
            controller.enqueue({ type: 'done', stopReason: 'end_turn' })
            controller.close()
          },
        })
      },
    }
    const service = new AgentHarnessService({
      agentStore: createAgentStore([agent]) as AgentStore,
      runtime,
    })

    const stream = await service.send({ agentId: agent.id, message: 'hi' })
    // Turn just kicked off — the activity tracker should report working.
    let listed = await service.listAgentsWithActivity()
    expect(listed[0]?.status).toBe('working')

    // Release the upstream so the lifecycle hook fires `notifyTurnEnded`,
    // then drain the consumer side.
    releaseUpstream()
    await collectStream(stream)
    listed = await service.listAgentsWithActivity()
    expect(listed[0]?.status).toBe('idle')
  })

  it('flips to error when a turn emits an error event', async () => {
    const agent: AgentDefinition = {
      id: 'err-1',
      name: 'err',
      adapter: 'claude',
      modelId: 'haiku',
      reasoningEffort: 'medium',
      permissionMode: 'approve-all',
      sessionKey: 'agent:err-1:main',
      createdAt: 1000,
      updatedAt: 1000,
    }
    const runtime: AgentRuntime = {
      async status() {
        return { state: 'ready' }
      },
      async listSessions() {
        return []
      },
      async getHistory() {
        return { agentId: agent.id, sessionId: 'main', items: [] }
      },
      async send() {
        return new ReadableStream<AgentStreamEvent>({
          start(controller) {
            controller.enqueue({ type: 'error', message: 'boom' })
            controller.close()
          },
        })
      },
    }
    const service = new AgentHarnessService({
      agentStore: createAgentStore([agent]) as AgentStore,
      runtime,
    })

    await collectStream(await service.send({ agentId: agent.id, message: 'x' }))
    const listed = await service.listAgentsWithActivity()
    expect(listed[0]?.status).toBe('error')
  })

  it('writes a per-agent Hermes config.yaml + .env when adapter=hermes and provider config complete', async () => {
    const agents: AgentDefinition[] = []
    const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-hermes-test-'))
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as AgentStore,
      runtime: stubRuntime(),
      browserosDir,
    })

    const agent = await service.createAgent({
      name: 'Hermes bot',
      adapter: 'hermes',
      providerType: 'openrouter',
      apiKey: 'sk-or-v1-test-key',
      modelId: 'anthropic/claude-haiku-4.5',
    })

    const homeDir = join(
      browserosDir,
      'vm',
      'hermes',
      'harness',
      agent.id,
      'home',
    )
    const yaml = readFileSync(join(homeDir, 'config.yaml'), 'utf8')
    const env = readFileSync(join(homeDir, '.env'), 'utf8')
    expect(yaml).toContain('"openrouter"')
    expect(yaml).toContain('"anthropic/claude-haiku-4.5"')
    expect(env).toContain('OPENROUTER_API_KEY=sk-or-v1-test-key')
  })

  it('rejects Hermes agent creation when apiKey is missing', async () => {
    const agents: AgentDefinition[] = []
    const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-hermes-test-'))
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as AgentStore,
      runtime: stubRuntime(),
      browserosDir,
    })

    await expect(
      service.createAgent({
        name: 'Hermes bot',
        adapter: 'hermes',
        providerType: 'openrouter',
        modelId: 'anthropic/claude-haiku-4.5',
      }),
    ).rejects.toThrow(/apiKey/i)
    expect(agents).toHaveLength(0)
  })

  it('rejects Hermes agent creation when providerType is missing', async () => {
    const agents: AgentDefinition[] = []
    const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-hermes-test-'))
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as AgentStore,
      runtime: stubRuntime(),
      browserosDir,
    })

    await expect(
      service.createAgent({ name: 'Hermes bot', adapter: 'hermes' }),
    ).rejects.toThrow(/providerType/i)
    expect(agents).toHaveLength(0)
  })

  it('rejects Hermes agent creation when modelId is missing', async () => {
    const agents: AgentDefinition[] = []
    const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-hermes-test-'))
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as AgentStore,
      runtime: stubRuntime(),
      browserosDir,
    })

    await expect(
      service.createAgent({
        name: 'Hermes bot',
        adapter: 'hermes',
        providerType: 'openrouter',
        apiKey: 'sk-or-v1-test-key',
      }),
    ).rejects.toThrow(/modelId/i)
    expect(agents).toHaveLength(0)
  })

  it('writes provider:custom + base_url for openai-compatible providers', async () => {
    const agents: AgentDefinition[] = []
    const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-hermes-test-'))
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as AgentStore,
      runtime: stubRuntime(),
      browserosDir,
    })

    const agent = await service.createAgent({
      name: 'Custom Hermes',
      adapter: 'hermes',
      providerType: 'openai-compatible',
      apiKey: 'sk-test',
      modelId: 'my-model',
      baseUrl: 'https://api.example.com/v1',
    })

    const homeDir = join(
      browserosDir,
      'vm',
      'hermes',
      'harness',
      agent.id,
      'home',
    )
    const yaml = readFileSync(join(homeDir, 'config.yaml'), 'utf8')
    const env = readFileSync(join(homeDir, '.env'), 'utf8')
    // Hermes has no provider key called "openai" — the canonical shape
    // for any OpenAI-compatible endpoint is `provider: custom` with
    // `base_url` set. Hermes then short-circuits provider lookup and
    // calls the URL directly using OPENAI_API_KEY.
    expect(yaml).toContain('"custom"')
    expect(yaml).toContain('"my-model"')
    expect(yaml).toContain('"https://api.example.com/v1"')
    expect(env).toContain('OPENAI_API_KEY=sk-test')
  })

  it('falls back to OpenAI default base_url for the openai provider type', async () => {
    const agents: AgentDefinition[] = []
    const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-hermes-test-'))
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as AgentStore,
      runtime: stubRuntime(),
      browserosDir,
    })

    const agent = await service.createAgent({
      name: 'OpenAI Hermes',
      adapter: 'hermes',
      providerType: 'openai',
      apiKey: 'sk-openai-test',
      modelId: 'gpt-4o-mini',
      // No baseUrl supplied — provider:custom still requires one,
      // so the mapping's defaultBaseUrl must take over.
    })

    const homeDir = join(
      browserosDir,
      'vm',
      'hermes',
      'harness',
      agent.id,
      'home',
    )
    const yaml = readFileSync(join(homeDir, 'config.yaml'), 'utf8')
    expect(yaml).toContain('"custom"')
    expect(yaml).toContain('"gpt-4o-mini"')
    expect(yaml).toContain('"https://api.openai.com/v1"')
  })

  it('rejects openai-compatible Hermes agent creation when baseUrl is missing', async () => {
    const agents: AgentDefinition[] = []
    const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-hermes-test-'))
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as AgentStore,
      runtime: stubRuntime(),
      browserosDir,
    })

    await expect(
      service.createAgent({
        name: 'Custom Hermes',
        adapter: 'hermes',
        providerType: 'openai-compatible',
        apiKey: 'sk-test',
        modelId: 'my-model',
      }),
    ).rejects.toThrow(/baseUrl/i)
    expect(agents).toHaveLength(0)
  })

  it('rejects Hermes agent creation when providerType is not in the supported set', async () => {
    const agents: AgentDefinition[] = []
    const browserosDir = mkdtempSync(join(tmpdir(), 'browseros-hermes-test-'))
    const service = new AgentHarnessService({
      agentStore: createAgentStore(agents) as AgentStore,
      runtime: stubRuntime(),
      browserosDir,
    })

    await expect(
      service.createAgent({
        name: 'Unknown Hermes',
        adapter: 'hermes',
        providerType: 'bedrock',
        apiKey: 'sk-test',
        modelId: 'm',
      }),
    ).rejects.toThrow(/not supported/i)
    expect(agents).toHaveLength(0)
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
  } satisfies Partial<AgentStore>
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
