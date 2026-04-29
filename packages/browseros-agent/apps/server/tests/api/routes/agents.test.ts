/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import { AGENT_HARNESS_LIMITS } from '@browseros/shared/constants/limits'
import { Hono } from 'hono'
import { createAgentRoutes } from '../../../src/api/routes/agents'
import {
  type ActiveTurnInfo,
  TurnRegistry,
} from '../../../src/lib/agents/active-turn-registry'
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
    expect(response.headers.get('X-Turn-Id')).toBeTruthy()
    const body = await response.text()
    // Frames now carry per-event seq ids so reconnects can resume.
    expect(body).toMatch(/^id: 0\ndata: /m)
    expect(body).toContain('data: [DONE]')
  })

  it('returns 409 when starting a turn while one is active', async () => {
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
    const route = createMountedRoutes([agent])

    // Block the runtime so the first turn stays "running".
    const blocking = createBlockingFakeService([agent])
    const blockingRoute = new Hono().route(
      '/agents',
      createAgentRoutes({ service: blocking }),
    )

    const first = blockingRoute.request('/agents/agent-1/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: 'hi' }),
    })

    // Yield so the first request reaches startTurn before the second
    // arrives.
    await new Promise((r) => setTimeout(r, 5))

    const second = await blockingRoute.request('/agents/agent-1/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: 'again' }),
    })
    expect(second.status).toBe(409)
    const body = await second.json()
    expect(body).toMatchObject({ error: 'Turn already active' })
    expect(typeof body.turnId).toBe('string')
    expect(body.attachUrl).toContain(`turnId=${body.turnId}`)

    // Unblock and drain the first.
    blocking._unblock()
    const firstResponse = await first
    await firstResponse.text()
    void route // keep type
  })

  it('reports the active turn via /chat/active and lets a client attach', async () => {
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
    const blocking = createBlockingFakeService([agent])
    const blockingRoute = new Hono().route(
      '/agents',
      createAgentRoutes({ service: blocking }),
    )

    // Kick off the first turn but don't read its body — that's the
    // "tab disconnected mid-turn" case.
    const first = blockingRoute.request('/agents/agent-1/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: 'hi' }),
    })
    await new Promise((r) => setTimeout(r, 5))

    const active = await blockingRoute.request('/agents/agent-1/chat/active')
    expect(active.status).toBe(200)
    const activeBody = await active.json()
    expect(activeBody.active).toMatchObject({
      agentId: 'agent-1',
      sessionId: 'main',
      status: 'running',
    })

    // Reattach as a fresh subscriber. Should get all buffered frames
    // when the runtime drains.
    const attachPromise = blockingRoute.request(
      `/agents/agent-1/chat/stream?turnId=${activeBody.active.turnId}`,
    )
    blocking._unblock()
    const attach = await attachPromise
    expect(attach.status).toBe(200)
    expect(attach.headers.get('X-Turn-Id')).toBe(activeBody.active.turnId)
    const attachBody = await attach.text()
    expect(attachBody).toContain('"type":"text_delta"')
    expect(attachBody).toContain('data: [DONE]')

    await (await first).text()
  })

  it('cancels an active turn via /chat/cancel', async () => {
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
    const blocking = createBlockingFakeService([agent])
    const blockingRoute = new Hono().route(
      '/agents',
      createAgentRoutes({ service: blocking }),
    )

    const first = blockingRoute.request('/agents/agent-1/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: 'hi' }),
    })
    await new Promise((r) => setTimeout(r, 5))

    const cancel = await blockingRoute.request('/agents/agent-1/chat/cancel', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ reason: 'user pressed stop' }),
    })
    expect(cancel.status).toBe(200)
    expect(await cancel.json()).toEqual({ cancelled: true })

    const text = await (await first).text()
    expect(text).toContain('"stopReason":"cancelled"')

    blocking._unblock()
  })

  it('returns 404 when attaching to an unknown turn', async () => {
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
    const response = await route.request(
      '/agents/agent-1/chat/stream?turnId=nope',
    )
    expect(response.status).toBe(404)
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
    expect(await list.json()).toEqual({ agents: [], gateway: null })
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
  // Per-test in-memory turn registry. The service-side fakes go through
  // it for the same reason real code does: keeps turn lifecycle decoupled
  // from the HTTP response, so reconnect/cancel/active-turn tests work
  // against the same primitives prod uses.
  const registry = new TurnRegistry({
    retainAfterDoneMs: 60_000,
    sweepIntervalMs: 60_000,
  })

  const fakeEvents: AgentStreamEvent[] = [
    { type: 'text_delta', text: 'Hello', stream: 'output' },
    { type: 'done', stopReason: 'end_turn' },
  ]

  return {
    async listAgents() {
      return agents
    },
    async listAgentsWithActivity() {
      // The route returns enriched agents in the listing response.
      // Tests don't care about activity values; default to `idle`/null.
      return agents.map((agent) => ({
        ...agent,
        status: 'idle' as const,
        lastUsedAt: null,
      }))
    },
    async getGatewayStatus() {
      // No openclaw provisioner wired in tests → `null` mirrors what
      // `AgentHarnessService.getGatewayStatus` does without one.
      return null
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
    async startTurn(input: { agentId: string }) {
      const turn = registry.register(input.agentId, 'main')
      const frames = registry.subscribe(turn.turnId, { fromSeq: -1 })!
      // Push the canned events asynchronously so subscribers actually
      // receive them through the stream, mirroring real runtime fan-out.
      queueMicrotask(() => {
        for (const event of fakeEvents) registry.pushEvent(turn.turnId, event)
      })
      return { turnId: turn.turnId, frames }
    },
    attachTurn(input: { turnId: string; lastSeq?: number }) {
      return registry.subscribe(input.turnId, { fromSeq: input.lastSeq ?? -1 })
    },
    getActiveTurn(agentId: string): ActiveTurnInfo | null {
      const t = registry.getActiveFor(agentId, 'main')
      return t ? registry.describe(t.turnId) : null
    },
    cancelTurn(input: { agentId: string; turnId?: string; reason?: string }) {
      const turnId =
        input.turnId ?? registry.getActiveFor(input.agentId, 'main')?.turnId
      if (!turnId) return false
      return registry.cancel(turnId, input.reason)
    },
    async send() {
      // Legacy shape, used by the sidepanel route only. Returns a flat
      // AgentStreamEvent stream.
      return createAgentStream(fakeEvents)
    },
    /** Test-only: lets tests await turn completion deterministically. */
    _registry: registry,
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

/**
 * Variant of `createFakeService` whose turn doesn't push frames until
 * `_unblock()` is called. Used by tests that need to observe the
 * "running" state — collisions, /chat/active discovery, cancel.
 */
function createBlockingFakeService(agents: AgentDefinition[]) {
  const registry = new TurnRegistry({
    retainAfterDoneMs: 60_000,
    sweepIntervalMs: 60_000,
  })
  const events: AgentStreamEvent[] = [
    { type: 'text_delta', text: 'Hello', stream: 'output' },
    { type: 'done', stopReason: 'end_turn' },
  ]
  let unblock: () => void = () => {}
  const gate = new Promise<void>((resolve) => {
    unblock = resolve
  })

  return {
    async listAgents() {
      return agents
    },
    async listAgentsWithActivity() {
      return agents.map((agent) => ({
        ...agent,
        status: 'idle' as const,
        lastUsedAt: null,
      }))
    },
    async getGatewayStatus() {
      return null
    },
    async createAgent() {
      throw new Error('not used in this test')
    },
    async getAgent(agentId: string) {
      return agents.find((a) => a.id === agentId) ?? null
    },
    async deleteAgent() {
      return false
    },
    async getHistory(agentId: string) {
      return { agentId, sessionId: 'main' as const, items: [] }
    },
    async startTurn(input: { agentId: string }) {
      const existing = registry.getActiveFor(input.agentId, 'main')
      if (existing) {
        const { TurnAlreadyActiveError } = await import(
          '../../../src/api/services/agents/agent-harness-service'
        )
        throw new TurnAlreadyActiveError(input.agentId, existing.turnId)
      }
      const turn = registry.register(input.agentId, 'main')
      const frames = registry.subscribe(turn.turnId, { fromSeq: -1 })!
      void (async () => {
        await gate
        for (const event of events) registry.pushEvent(turn.turnId, event)
      })()
      return { turnId: turn.turnId, frames }
    },
    attachTurn(input: { turnId: string; lastSeq?: number }) {
      return registry.subscribe(input.turnId, { fromSeq: input.lastSeq ?? -1 })
    },
    getActiveTurn(agentId: string): ActiveTurnInfo | null {
      const t = registry.getActiveFor(agentId, 'main')
      return t ? registry.describe(t.turnId) : null
    },
    cancelTurn(input: { agentId: string; turnId?: string; reason?: string }) {
      const turnId =
        input.turnId ?? registry.getActiveFor(input.agentId, 'main')?.turnId
      if (!turnId) return false
      return registry.cancel(turnId, input.reason)
    },
    async send() {
      return createAgentStream(events)
    },
    _unblock: () => unblock(),
  }
}
