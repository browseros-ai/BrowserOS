/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { AgentHarnessService } from '../../../../src/api/services/agents/agent-harness-service'
import type {
  AgentDefinition,
  AgentSessionId,
} from '../../../../src/lib/agents/agent-types'
import type { AgentStore } from '../../../../src/lib/agents/storage/agent-store'
import {
  type TurnFrame,
  TurnRegistry,
} from '../../../../src/lib/agents/turns/active-turn-registry'
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

  it('reads history from a requested session id', async () => {
    const sessionId = '00000000-0000-4000-8000-000000000001'
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
        return { agentId: agent.id, sessionId: input.sessionId, items: [] }
      },
      async send() {
        return new ReadableStream<AgentStreamEvent>()
      },
    }
    const service = new AgentHarnessService({
      agentStore: createAgentStore([agent]) as AgentStore,
      runtime,
    })

    await service.getHistory(agent.id, sessionId)

    expect(runtimeInputs).toEqual([{ agent, sessionId }])
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

  it('shows latest sidepanel session errors on the activity row', async () => {
    const sessionId = '00000000-0000-4000-8000-000000000001'
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
    const runtime: AgentRuntime = {
      async status() {
        return { state: 'ready' }
      },
      async listSessions() {
        return []
      },
      async getHistory(input) {
        return {
          agentId: input.agent.id,
          sessionId: input.sessionId,
          items: [],
        }
      },
      async send() {
        return new ReadableStream<AgentStreamEvent>({
          start(controller) {
            controller.enqueue({ type: 'error', message: 'sidepanel failed' })
            controller.close()
          },
        })
      },
    }
    const service = new AgentHarnessService({
      agentStore: createAgentStore([agent]) as AgentStore,
      runtime,
    })

    const turn = await service.startTurn({
      agentId: agent.id,
      sessionId,
      message: 'sidepanel turn',
    })
    await collectFrameStream(turn.frames)
    const listed = await service.listAgentsWithActivity()
    expect(listed[0]?.status).toBe('error')
    expect(listed[0]?.latestSessionId).toBe(sessionId)
    expect(listed[0]?.lastError).toBe('sidepanel failed')
  })

  it('prefers newer live activity over an older persisted row snapshot', async () => {
    const sessionId = '00000000-0000-4000-8000-000000000001'
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
    const held = createHeldRuntime()
    const runtime: AgentRuntime = {
      ...held.runtime,
      async getLatestRowSnapshot() {
        return {
          sessionId: 'main',
          cwd: null,
          lastUsedAt: 1,
          lastUserMessage: 'old main prompt',
          tokens: null,
        }
      },
    }
    const service = new AgentHarnessService({
      agentStore: createAgentStore([agent]) as AgentStore,
      runtime,
    })

    const turn = await service.startTurn({
      agentId: agent.id,
      sessionId,
      message: 'new sidepanel prompt',
    })
    const frames = collectFrameStream(turn.frames)
    const listed = await service.listAgentsWithActivity()

    expect(listed[0]?.status).toBe('working')
    expect(listed[0]?.latestSessionId).toBe(sessionId)
    expect(listed[0]?.activeTurnId).toBe(turn.turnId)
    expect(listed[0]?.lastUserMessage).toBe('new sidepanel prompt')
    expect(listed[0]?.lastUsedAt).toBeGreaterThan(1)

    held.release(sessionId)
    await frames
  })

  it('runs concurrent turns for different sessions and blocks duplicates per session', async () => {
    const sessionId = '00000000-0000-4000-8000-000000000001'
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
    const held = createHeldRuntime()
    const service = new AgentHarnessService({
      agentStore: createAgentStore([agent]) as AgentStore,
      runtime: held.runtime,
    })

    const main = await service.startTurn({
      agentId: agent.id,
      sessionId: 'main',
      message: 'main turn',
    })
    const sidepanel = await service.startTurn({
      agentId: agent.id,
      sessionId,
      message: 'sidepanel turn',
    })

    expect(held.inputs.map((input) => input.sessionId)).toEqual([
      'main',
      sessionId,
    ])
    await expect(
      service.startTurn({
        agentId: agent.id,
        sessionId,
        message: 'duplicate',
      }),
    ).rejects.toThrow('already has an active turn')

    held.release('main')
    await collectFrameStream(main.frames)
    let listed = await service.listAgentsWithActivity()
    expect(listed[0]?.status).toBe('working')
    expect(listed[0]?.latestSessionId).toBe(sessionId)

    held.release(sessionId)
    await collectFrameStream(sidepanel.frames)
    listed = await service.listAgentsWithActivity()
    expect(listed[0]?.status).toBe('idle')
  })

  it('drains queued messages into the queued session', async () => {
    const sessionId = '00000000-0000-4000-8000-000000000001'
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
    const held = createHeldRuntime()
    const service = new AgentHarnessService({
      agentStore: createAgentStore([agent]) as AgentStore,
      runtime: held.runtime,
    })

    const first = await service.startTurn({
      agentId: agent.id,
      sessionId,
      message: 'first',
    })
    const queued = await service.enqueueMessage({
      agentId: agent.id,
      sessionId,
      message: 'second',
    })

    expect(queued.sessionId).toBe(sessionId)
    held.release(sessionId)
    await collectFrameStream(first.frames)
    await waitFor(() => held.inputs.length === 2)

    expect(held.inputs.map((input) => input.sessionId)).toEqual([
      sessionId,
      sessionId,
    ])
    held.release(sessionId)
  })

  it('strips browser-context scaffolding from the active-turn prompt', () => {
    const { registry, service } = serviceWithRegistry()
    registry.register('agent-1', 'main', {
      prompt: [
        '## Browser Context',
        '**Window ID:** 1995357486',
        '**Active Tab:** Tab 1995357512 (Page ID: 3) - "BrowserOS" (chrome://newtab/)',
        '',
        '---',
        '',
        '<USER_QUERY>',
        'Open amazon.com in current tab and add sensodyne toothpaste to cart',
        '</USER_QUERY>',
      ].join('\n'),
    })

    expect(service.getActiveTurn('agent-1')?.prompt).toBe(
      'Open amazon.com in current tab and add sensodyne toothpaste to cart',
    )
  })

  it('passes a null active-turn prompt through unchanged', () => {
    const { registry, service } = serviceWithRegistry()
    registry.register('agent-1', 'main', { prompt: null })

    expect(service.getActiveTurn('agent-1')?.prompt).toBeNull()
  })

  it('leaves an already-clean active-turn prompt unchanged', () => {
    const { registry, service } = serviceWithRegistry()
    registry.register('agent-1', 'main', { prompt: 'plain question' })

    expect(service.getActiveTurn('agent-1')?.prompt).toBe('plain question')
  })
})

function serviceWithRegistry(): {
  registry: TurnRegistry
  service: AgentHarnessService
} {
  const registry = new TurnRegistry()
  const service = new AgentHarnessService({
    agentStore: createAgentStore([]) as AgentStore,
    runtime: stubRuntime(),
    turnRegistry: registry,
  })
  return { registry, service }
}

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

function createHeldRuntime(): {
  runtime: AgentRuntime
  inputs: Array<Parameters<AgentRuntime['send']>[0]>
  release(sessionId: AgentSessionId): void
} {
  const inputs: Array<Parameters<AgentRuntime['send']>[0]> = []
  const releases = new Map<AgentSessionId, () => void>()
  return {
    inputs,
    release(sessionId) {
      const release = releases.get(sessionId)
      if (!release) throw new Error(`No held stream for ${sessionId}`)
      release()
      releases.delete(sessionId)
    },
    runtime: {
      async status() {
        return { state: 'ready' }
      },
      async listSessions() {
        return []
      },
      async getHistory(input) {
        return {
          agentId: input.agent.id,
          sessionId: input.sessionId,
          items: [],
        }
      },
      async send(input) {
        inputs.push(input)
        const gate = new Promise<void>((resolve) => {
          releases.set(input.sessionId, resolve)
        })
        return new ReadableStream<AgentStreamEvent>({
          async start(controller) {
            controller.enqueue({
              type: 'text_delta',
              text: `started ${input.sessionId}`,
              stream: 'output',
            })
            await gate
            controller.enqueue({ type: 'done', stopReason: 'end_turn' })
            controller.close()
          },
        })
      },
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

async function collectFrameStream(
  stream: ReadableStream<TurnFrame>,
): Promise<TurnFrame[]> {
  const reader = stream.getReader()
  const frames: TurnFrame[] = []
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      frames.push(value)
    }
  } finally {
    reader.releaseLock()
  }
  return frames
}

async function waitFor(
  predicate: () => boolean,
  timeoutMs = 500,
): Promise<void> {
  const started = Date.now()
  while (!predicate()) {
    if (Date.now() - started > timeoutMs) {
      throw new Error('Timed out waiting for condition')
    }
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
}
