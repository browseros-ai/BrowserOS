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
import type { AgentStreamEvent } from '../../../src/lib/agents/types'

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

  it('streams created-agent sidepanel chat through the persisted agent', async () => {
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
    const service = createFakeService([agent])
    const route = new Hono().route(
      '/agents',
      createAgentRoutes({
        service,
        browser: {
          async resolveTabIds(tabIds: number[]) {
            return new Map(tabIds.map((tabId) => [tabId, tabId + 100]))
          },
        },
      }),
    )

    const response = await route.request('/agents/agent-1/sidepanel/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        ...validCreatedAgentSidepanelBody(),
        adapter: 'openclaw',
        modelId: 'ignored-client-model',
        reasoningEffort: 'ignored-client-effort',
        userSystemPrompt: 'Always be concise.',
        userWorkingDir: '/tmp/work',
        browserContext: {
          activeTab: { id: 1, url: 'https://example.com', title: 'Example' },
        },
        selectedText: 'selected text',
        selectedTextSource: {
          url: 'https://example.com',
          title: 'Example',
        },
      }),
    })

    expect(response.status).toBe(200)
    expect(response.headers.get('Content-Type')).toContain('text/event-stream')
    expect(response.headers.get('x-vercel-ai-ui-message-stream')).toBe('v1')
    expect(await response.text()).toContain('"type":"text-delta"')
    expect(service._lastStartTurnInput).toMatchObject({
      agentId: 'agent-1',
      cwd: '/tmp/work',
    })
    expect(service._lastStartTurnInput?.message).toContain('Always be concise.')
    expect(service._lastStartTurnInput?.message).toContain(
      'Tab 1 (Page ID: 101) - "Example" (https://example.com)',
    )
    expect(service._lastStartTurnInput?.message).toContain(
      '<selected_text (from "Example"',
    )
    expect(service._lastStartTurnInput?.message).toContain(
      'selected text\n</selected_text>',
    )
    expect(service._lastStartTurnInput?.message).toContain(
      '<USER_QUERY>\nhi\n</USER_QUERY>',
    )

    const list = await route.request('/agents')
    expect(await list.json()).toMatchObject({
      agents: [{ id: 'agent-1', adapter: 'codex', modelId: 'gpt-5.5' }],
    })
  })

  it('rejects invalid created-agent sidepanel chat requests', async () => {
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

    const unknown = await route.request('/agents/missing/sidepanel/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(validCreatedAgentSidepanelBody()),
    })
    expect(unknown.status).toBe(404)

    for (const { patch, error } of [
      {
        patch: { conversationId: 'not-a-uuid' },
        error: 'conversationId must be a UUID',
      },
      { patch: { message: '   ' }, error: 'Message is required' },
      {
        patch: { browserContext: { activeTab: { id: 'bad' } } },
        error: 'Invalid browserContext',
      },
      {
        patch: { selectedTextSource: { url: 123, title: 'Example' } },
        error: 'Invalid selectedTextSource',
      },
    ]) {
      const response = await route.request('/agents/agent-1/sidepanel/chat', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          ...validCreatedAgentSidepanelBody(),
          ...patch,
        }),
      })

      expect(response.status).toBe(400)
      expect(await response.json()).toEqual({ error })
    }
  })

  it('cancels the created-agent sidepanel turn when the request is aborted', async () => {
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
    const route = new Hono().route(
      '/agents',
      createAgentRoutes({ service: blocking }),
    )
    const abortController = new AbortController()

    const response = await route.request('/agents/agent-1/sidepanel/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      signal: abortController.signal,
      body: JSON.stringify(validCreatedAgentSidepanelBody()),
    })

    expect(response.status).toBe(200)
    abortController.abort('sidepanel closed')
    await new Promise((r) => setTimeout(r, 0))
    expect(blocking._cancelCalls).toEqual([
      {
        agentId: 'agent-1',
        reason: 'sidepanel stream cancelled',
      },
    ])
    blocking._unblock()
  })

  it('returns 409 when a created-agent sidepanel turn is already active', async () => {
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
    const route = new Hono().route(
      '/agents',
      createAgentRoutes({ service: blocking }),
    )

    const first = route.request('/agents/agent-1/sidepanel/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(validCreatedAgentSidepanelBody()),
    })
    await new Promise((r) => setTimeout(r, 5))

    const second = await route.request('/agents/agent-1/sidepanel/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(validCreatedAgentSidepanelBody()),
    })

    expect(second.status).toBe(409)
    const body = await second.json()
    expect(body).toMatchObject({ error: 'Turn already active' })
    expect(typeof body.turnId).toBe('string')
    expect(body.attachUrl).toContain(`turnId=${body.turnId}`)

    blocking._unblock()
    await (await first).text()
  })

  it('does not expose the legacy virtual sidepanel ACP chat route', async () => {
    const route = createMountedRoutes([])

    const response = await route.request('/agents/sidepanel/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        conversationId: '00000000-0000-4000-8000-000000000001',
        adapter: 'codex',
        modelId: 'gpt-5.5',
        reasoningEffort: 'medium',
        message: 'hi',
      }),
    })

    expect(response.status).toBe(404)
  })

  it('PATCH /:agentId updates pinned + name and rejects empty patches', async () => {
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

    const pinned = await route.request('/agents/agent-1', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ pinned: true }),
    })
    expect(pinned.status).toBe(200)
    expect(await pinned.json()).toMatchObject({
      agent: { id: 'agent-1', pinned: true },
    })

    const renamed = await route.request('/agents/agent-1', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'Renamed' }),
    })
    expect(renamed.status).toBe(200)
    expect(await renamed.json()).toMatchObject({
      agent: { id: 'agent-1', name: 'Renamed' },
    })

    const empty = await route.request('/agents/agent-1', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    })
    expect(empty.status).toBe(400)

    const unknown = await route.request('/agents/missing', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ pinned: false }),
    })
    expect(unknown.status).toBe(404)
  })

  it('queues + lists + removes messages for an agent', async () => {
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

    const enqueueA = await route.request('/agents/agent-1/queue', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: 'first', attachments: [] }),
    })
    expect(enqueueA.status).toBe(200)
    const enqueuedA = await enqueueA.json()
    expect(enqueuedA.queued).toMatchObject({ message: 'first' })

    const enqueueB = await route.request('/agents/agent-1/queue', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: 'second' }),
    })
    expect(enqueueB.status).toBe(200)
    const enqueuedB = await enqueueB.json()

    const listed = await route.request('/agents/agent-1/queue')
    expect(listed.status).toBe(200)
    const listedBody = await listed.json()
    expect(listedBody.queue.map((q: { message: string }) => q.message)).toEqual(
      ['first', 'second'],
    )

    const removed = await route.request(
      `/agents/agent-1/queue/${enqueuedA.queued.id}`,
      { method: 'DELETE' },
    )
    expect(removed.status).toBe(200)
    expect(await removed.json()).toEqual({ removed: true })

    const afterRemove = await route.request('/agents/agent-1/queue')
    expect((await afterRemove.json()).queue).toEqual([
      expect.objectContaining({ id: enqueuedB.queued.id, message: 'second' }),
    ])

    const removeMissing = await route.request(
      '/agents/agent-1/queue/does-not-exist',
      { method: 'DELETE' },
    )
    expect(removeMissing.status).toBe(404)
  })

  it('rejects empty queue messages and unknown agents', async () => {
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

    const empty = await route.request('/agents/agent-1/queue', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: '   ' }),
    })
    expect(empty.status).toBe(400)

    const unknown = await route.request('/agents/missing/queue', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ message: 'hi' }),
    })
    expect(unknown.status).toBe(404)
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
  let lastStartTurnInput:
    | { agentId: string; message?: string; cwd?: string }
    | undefined
  const queues = new Map<
    string,
    Array<{
      id: string
      createdAt: number
      message: string
      attachments?: ReadonlyArray<{ mediaType: string; data: string }>
    }>
  >()

  return {
    get _lastStartTurnInput() {
      return lastStartTurnInput
    },
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
    async updateAgent(
      agentId: string,
      patch: { name?: string; pinned?: boolean },
    ) {
      const index = agents.findIndex((agent) => agent.id === agentId)
      if (index < 0) return null
      const next = {
        ...agents[index],
        ...(patch.name !== undefined ? { name: patch.name.trim() } : {}),
        ...(patch.pinned !== undefined ? { pinned: patch.pinned } : {}),
        updatedAt: Date.now(),
      }
      agents[index] = next
      return next
    },
    async getHistory(agentId: string) {
      return {
        agentId,
        sessionId: 'main' as const,
        items: [],
      }
    },
    async startTurn(input: {
      agentId: string
      message?: string
      cwd?: string
    }) {
      if (!agents.some((agent) => agent.id === input.agentId)) {
        const { UnknownAgentError } = await import(
          '../../../src/api/services/agents/agent-harness-service'
        )
        throw new UnknownAgentError(input.agentId)
      }
      lastStartTurnInput = input
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
    async enqueueMessage(input: {
      agentId: string
      message: string
      attachments?: ReadonlyArray<{ mediaType: string; data: string }>
    }) {
      if (!agents.some((a) => a.id === input.agentId)) {
        const { UnknownAgentError } = await import(
          '../../../src/api/services/agents/agent-harness-service'
        )
        throw new UnknownAgentError(input.agentId)
      }
      const queued = {
        id: `q-${Math.random().toString(36).slice(2, 10)}`,
        createdAt: Date.now(),
        message: input.message,
        attachments: input.attachments,
      }
      const list = queues.get(input.agentId) ?? []
      list.push(queued)
      queues.set(input.agentId, list)
      return queued
    },
    async removeQueuedMessage(input: { agentId: string; messageId: string }) {
      const list = queues.get(input.agentId)
      if (!list) return false
      const next = list.filter((entry) => entry.id !== input.messageId)
      if (next.length === list.length) return false
      if (next.length === 0) queues.delete(input.agentId)
      else queues.set(input.agentId, next)
      return true
    },
    async listQueuedMessages(agentId: string) {
      return queues.get(agentId)?.slice() ?? []
    },
    /** Test-only: lets tests await turn completion deterministically. */
    _registry: registry,
    _queues: queues,
  }
}

function validCreatedAgentSidepanelBody() {
  return {
    conversationId: '00000000-0000-4000-8000-000000000001',
    message: 'hi',
  }
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
  const cancelCalls: Array<{ agentId: string; reason?: string }> = []
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
    async updateAgent() {
      return null
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
      cancelCalls.push({ agentId: input.agentId, reason: input.reason })
      const turnId =
        input.turnId ?? registry.getActiveFor(input.agentId, 'main')?.turnId
      if (!turnId) return false
      return registry.cancel(turnId, input.reason)
    },
    async enqueueMessage() {
      throw new Error('not used in this test')
    },
    async removeQueuedMessage() {
      return false
    },
    async listQueuedMessages() {
      return []
    },
    _unblock: () => unblock(),
    _cancelCalls: cancelCalls,
  }
}
