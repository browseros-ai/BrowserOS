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
})

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
    async delete() {
      return true
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
