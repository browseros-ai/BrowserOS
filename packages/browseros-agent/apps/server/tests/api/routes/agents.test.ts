/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { AGENT_HARNESS_LIMITS } from '@browseros/shared/constants/limits'
import { Hono } from 'hono'
import { createAgentRoutes } from '../../../src/api/routes/agents'
import type { AgentDefinition } from '../../../src/lib/agents/agent-types'
import type {
  AgentPromptInput,
  AgentRuntime,
  AgentStreamEvent,
} from '../../../src/lib/agents/types'

describe('createAgentRoutes', () => {
  it('creates and lists harness agents', async () => {
    const agents: AgentDefinition[] = []
    const route = createMountedRoutes(agents)
    const created = await route.request('/agents', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        name: 'Review bot',
        adapter: 'codex',
        modelId: 'gpt-5.5',
        reasoningEffort: 'medium',
      }),
    })

    expect(created.status).toBe(200)
    expect(await created.json()).toMatchObject({
      agent: { name: 'Review bot', adapter: 'codex' },
    })

    const list = await route.request('/agents')
    expect(await list.json()).toMatchObject({
      agents: [{ name: 'Review bot', adapter: 'codex' }],
    })
  })

  it('streams chat for an agent main session', async () => {
    const route = createMountedRoutes([
      {
        id: 'agent-1',
        name: 'Review bot',
        adapter: 'codex',
        modelId: 'gpt-5.5',
        reasoningEffort: 'medium',
        permissionMode: 'approve-all',
        sessionKey: 'agent:agent-1:main',
        createdAt: 1000,
        updatedAt: 1000,
      },
    ])

    const response = await route.request('/agents/agent-1/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: 'hi' }),
    })

    expect(response.status).toBe(200)
    expect(response.headers.get('X-Session-Id')).toBe('main')
    expect(await response.text()).toContain('data: [DONE]')
  })

  it('streams sidepanel ACP chat as an AI SDK UI message stream', async () => {
    const conversationId = '00000000-0000-4000-8000-000000000001'
    let sentInput: AgentPromptInput | undefined
    const abortController = new AbortController()
    const route = createMountedRoutes([], {
      browser: {
        async resolveTabIds(tabIds: number[]) {
          return new Map(tabIds.map((tabId) => [tabId, tabId + 100]))
        },
      },
      runtime: createFakeRuntime(async (input) => {
        sentInput = input
        return createAgentStream([
          { type: 'text_delta', text: 'Hello', stream: 'output' },
          { type: 'done', stopReason: 'end_turn' },
        ])
      }),
    })

    const response = await route.request('/agents/sidepanel/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      signal: abortController.signal,
      body: JSON.stringify({
        conversationId,
        adapter: 'codex',
        modelId: 'gpt-5.5',
        reasoningEffort: 'medium',
        message: 'hi',
        userWorkingDir: '/tmp/work',
        browserContext: {
          activeTab: { id: 1, url: 'https://example.com', title: 'Example' },
        },
      }),
    })

    expect(response.status).toBe(200)
    expect(response.headers.get('Content-Type')).toContain('text/event-stream')
    expect(await response.text()).toContain('"type":"text-delta"')
    expect(sentInput?.agent).toMatchObject({
      id: `sidepanel:${conversationId}`,
      adapter: 'codex',
      modelId: 'gpt-5.5',
      reasoningEffort: 'medium',
      permissionMode: 'approve-all',
      sessionKey: `sidepanel:${conversationId}:codex:gpt-5.5:medium`,
    })
    expect(sentInput?.cwd).toBe('/tmp/work')
    expect(sentInput?.message).toContain(
      'Tab 1 (Page ID: 101) - "Example" (https://example.com)',
    )
    expect(sentInput?.message).toContain('<USER_QUERY>\nhi\n</USER_QUERY>')
    expect(sentInput?.signal).toBe(abortController.signal)

    const list = await route.request('/agents')
    expect(await list.json()).toEqual({ agents: [] })
  })

  it('rejects invalid sidepanel ACP chat requests', async () => {
    const route = createMountedRoutes([])

    for (const { patch, error } of [
      {
        patch: { conversationId: 'not-a-uuid' },
        error: 'conversationId must be a UUID',
      },
      { patch: { adapter: 'openai' }, error: 'Invalid adapter' },
      { patch: { modelId: 'unknown-model' }, error: 'Invalid modelId' },
      { patch: { reasoningEffort: 'turbo' }, error: 'Invalid reasoningEffort' },
      { patch: { message: '   ' }, error: 'Message is required' },
    ]) {
      const response = await route.request('/agents/sidepanel/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          ...validSidepanelAcpBody(),
          ...patch,
        }),
      })

      expect(response.status).toBe(400)
      expect(await response.json()).toEqual({ error })
    }
  })

  it('rejects overlong agent names', async () => {
    const route = createMountedRoutes([])
    const response = await route.request('/agents', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        name: 'a'.repeat(AGENT_HARNESS_LIMITS.AGENT_NAME_MAX_CHARS + 1),
        adapter: 'codex',
      }),
    })

    expect(response.status).toBe(400)
    expect(await response.json()).toEqual({
      error: `Name must be ${AGENT_HARNESS_LIMITS.AGENT_NAME_MAX_CHARS} characters or fewer`,
    })
  })
})

function createMountedRoutes(
  agents: AgentDefinition[],
  deps: {
    runtime?: AgentRuntime
    browser?: { resolveTabIds(tabIds: number[]): Promise<Map<number, number>> }
  } = {},
) {
  return new Hono().route(
    '/agents',
    createAgentRoutes({ service: createFakeService(agents), ...deps }),
  )
}

function createFakeService(agents: AgentDefinition[]) {
  return {
    async listAgents() {
      return agents
    },
    async createAgent(input: {
      name: string
      adapter: 'claude' | 'codex' | 'openclaw'
      modelId?: string
      reasoningEffort?: string
    }) {
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
    async getAgent(agentId: string) {
      return agents.find((agent) => agent.id === agentId) ?? null
    },
    async deleteAgent(agentId: string) {
      const index = agents.findIndex((agent) => agent.id === agentId)
      if (index < 0) return false
      agents.splice(index, 1)
      return true
    },
    async getHistory(agentId: string) {
      return {
        agentId,
        sessionId: 'main' as const,
        items: [],
      }
    },
    async send() {
      return createAgentStream([
        {
          type: 'text_delta',
          text: 'Hello',
          stream: 'output',
        },
        { type: 'done', stopReason: 'end_turn' },
      ])
    },
  }
}

function validSidepanelAcpBody() {
  return {
    conversationId: '00000000-0000-4000-8000-000000000001',
    adapter: 'codex',
    modelId: 'gpt-5.5',
    reasoningEffort: 'medium',
    message: 'hi',
  }
}

function createFakeRuntime(
  send: (input: AgentPromptInput) => Promise<ReadableStream<AgentStreamEvent>>,
): AgentRuntime {
  return {
    async status() {
      return { state: 'ready' }
    },
    async listSessions(agent) {
      return [{ agentId: agent.id, id: 'main', updatedAt: agent.updatedAt }]
    },
    async getHistory(input) {
      return { agentId: input.agent.id, sessionId: 'main', items: [] }
    },
    send,
  }
}

function createAgentStream(
  events: AgentStreamEvent[],
): ReadableStream<AgentStreamEvent> {
  return new ReadableStream<AgentStreamEvent>({
    start(controller) {
      for (const event of events) controller.enqueue(event)
      controller.close()
    },
  })
}
