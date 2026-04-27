/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it, mock } from 'bun:test'
import { OpenClawSessionNotFoundError } from '../../../src/api/services/openclaw/errors'
import { UnsupportedOpenClawProviderError } from '../../../src/api/services/openclaw/openclaw-provider-map'

describe('createOpenClawRoutes', () => {
  afterEach(() => {
    mock.restore()
  })

  it('preserves BrowserOS SSE framing and normalizes recursive session keys for chat', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const actualMonitoringService = await import(
      '../../../src/monitoring/service'
    )
    const chatStream = mock(
      async () =>
        new ReadableStream({
          start(controller) {
            controller.enqueue({
              type: 'text-delta',
              data: { text: 'Hello' },
            })
            controller.enqueue({
              type: 'done',
              data: { text: 'Hello' },
            })
            controller.close()
          },
        }),
    )

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () =>
        ({
          chatStream,
        }) as never,
    }))

    mock.module('../../../src/monitoring/service', () => ({
      ...actualMonitoringService,
      getMonitoringService: () =>
        ({
          waitForSessionFree: async () => undefined,
          startSession: async () => ({
            monitoringSessionId: 'm-1',
            agentId: 'research',
            sessionKey: 'session-123',
            originalPrompt: 'hi',
            chatHistory: [],
            startedAt: new Date().toISOString(),
            source: 'openclaw-agent-chat' as const,
          }),
          finalizeSession: async () => undefined,
        }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/agents/research/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        message: 'hi',
        sessionKey:
          'agent:research:openai-user:browseros:research:agent:research:openai-user:browseros:research:session-123',
      }),
    })

    expect(response.status).toBe(200)
    expect(response.headers.get('Content-Type')).toContain('text/event-stream')
    expect(response.headers.get('X-Session-Key')).toBe('session-123')
    expect(chatStream).toHaveBeenCalledWith(
      'research',
      'session-123',
      'hi',
      [],
      {
        messageParts: undefined,
      },
    )
    expect(await response.text()).toBe(
      'data: {"type":"text-delta","data":{"text":"Hello"}}\n\n' +
        'data: {"type":"done","data":{"text":"Hello"}}\n\n' +
        'data: [DONE]\n\n',
    )
  })

  it('passes prior chat history through to the OpenClaw chat stream', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const actualMonitoringService = await import(
      '../../../src/monitoring/service'
    )
    const chatStream = mock(
      async () =>
        new ReadableStream({
          start(controller) {
            controller.enqueue({
              type: 'done',
              data: { text: 'Done' },
            })
            controller.close()
          },
        }),
    )

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () =>
        ({
          chatStream,
        }) as never,
    }))

    mock.module('../../../src/monitoring/service', () => ({
      ...actualMonitoringService,
      getMonitoringService: () =>
        ({
          waitForSessionFree: async () => undefined,
          startSession: async () => ({
            monitoringSessionId: 'm-2',
            agentId: 'research',
            sessionKey: 'session-456',
            originalPrompt: 'Summarize what is blocked',
            chatHistory: [],
            startedAt: new Date().toISOString(),
            source: 'openclaw-agent-chat' as const,
          }),
          finalizeSession: async () => undefined,
        }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()
    const history = [
      { role: 'user' as const, content: 'Find my open tasks' },
      { role: 'assistant' as const, content: 'I am checking Linear now.' },
    ]

    const response = await route.request('/agents/research/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        message: 'Summarize what is blocked',
        sessionKey: 'session-456',
        history,
      }),
    })

    expect(response.status).toBe(200)
    expect(chatStream).toHaveBeenCalledWith(
      'research',
      'session-456',
      'Summarize what is blocked',
      history,
      { messageParts: undefined },
    )
  })

  it('returns 503 when waitForSessionFree times out for a busy agent', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const actualMonitoringService = await import(
      '../../../src/monitoring/service'
    )
    const chatStream = mock(async () => new ReadableStream())

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () =>
        ({
          chatStream,
        }) as never,
    }))

    mock.module('../../../src/monitoring/service', () => ({
      ...actualMonitoringService,
      getMonitoringService: () =>
        ({
          waitForSessionFree: async () => {
            throw new Error(
              'Timed out waiting for agent "research" to become free after 30000ms',
            )
          },
        }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/agents/research/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        message: 'hi',
        sessionKey: 'session-789',
      }),
    })

    expect(response.status).toBe(503)
    expect(chatStream).not.toHaveBeenCalled()
    expect(await response.json()).toEqual({
      error:
        'Timed out waiting for agent "research" to become free after 30000ms',
    })
  })

  it('returns 400 for unsupported provider payloads', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const updateProviderKeys = mock(async () => {
      throw new UnsupportedOpenClawProviderError('google')
    })

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () =>
        ({
          updateProviderKeys,
        }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/providers', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        providerType: 'google',
        apiKey: 'google-key',
      }),
    })

    expect(response.status).toBe(400)
    expect(updateProviderKeys).toHaveBeenCalledWith({
      providerType: 'google',
      apiKey: 'google-key',
    })
    expect(await response.json()).toEqual({
      error: 'Unsupported OpenClaw provider: google',
    })
  })

  it('returns a non-restarting response when only the default model changes', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const updateProviderKeys = mock(async () => ({
      restarted: false,
      modelUpdated: true,
    }))

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () =>
        ({
          updateProviderKeys,
        }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/providers', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        providerType: 'openai',
        apiKey: 'sk-test',
        modelId: 'gpt-5.4-mini',
      }),
    })

    expect(response.status).toBe(200)
    expect(updateProviderKeys).toHaveBeenCalledWith({
      providerType: 'openai',
      apiKey: 'sk-test',
      modelId: 'gpt-5.4-mini',
    })
    expect(await response.json()).toEqual({
      status: 'updated',
      message: 'Provider updated without a restart',
    })
  })

  it('does not expose a roles route', async () => {
    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/roles')

    expect(response.status).toBe(404)
  })

  it('returns OpenClaw sessions for an agent', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const listSessions = mock(async () => [
      {
        key: 'openai-user:browseros:main:session-1',
        updatedAt: 20,
        sessionId: 'session-1',
        agentId: 'main',
        kind: 'chat',
        source: 'user-chat',
      },
    ])

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () => ({ listSessions }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/agents/main/sessions?limit=1')

    expect(response.status).toBe(200)
    expect(listSessions).toHaveBeenCalledWith('main')
    expect(await response.json()).toEqual({
      agentId: 'main',
      sessions: [
        {
          key: 'openai-user:browseros:main:session-1',
          updatedAt: 20,
          sessionId: 'session-1',
          agentId: 'main',
          kind: 'chat',
          source: 'user-chat',
        },
      ],
    })
  })

  it('returns the resolved active OpenClaw session for an agent', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const resolveAgentSession = mock(async () => ({
      agentId: 'main',
      exists: true,
      sessionKey: 'session-1',
      session: {
        key: 'session-1',
        updatedAt: 20,
        sessionId: 'session-1',
        agentId: 'main',
        kind: 'chat',
        source: 'other',
      },
    }))

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () => ({ resolveAgentSession }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/agents/main/session')

    expect(response.status).toBe(200)
    expect(resolveAgentSession).toHaveBeenCalledWith('main')
    expect(await response.json()).toEqual({
      agentId: 'main',
      exists: true,
      sessionKey: 'session-1',
      session: {
        key: 'session-1',
        updatedAt: 20,
        sessionId: 'session-1',
        agentId: 'main',
        kind: 'chat',
        source: 'other',
      },
    })
  })

  it('returns a normalized OpenClaw history page for an agent', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const getAgentHistoryPage = mock(async () => ({
      agentId: 'main',
      sessionKey: 'session-1',
      session: {
        key: 'session-1',
        updatedAt: 20,
        sessionId: 'session-1',
        agentId: 'main',
        kind: 'chat',
        source: 'other',
      },
      items: [
        {
          id: 'session-1:0',
          role: 'user',
          text: 'Hello',
          timestamp: 1,
          messageSeq: 0,
          sessionKey: 'session-1',
          source: 'other',
        },
      ],
      page: {
        cursor: 'older-cursor',
        hasMore: true,
        limit: 25,
      },
    }))

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () => ({ getAgentHistoryPage }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request(
      '/agents/main/history?sessionKey=session-1&cursor=abc&limit=25',
    )

    expect(response.status).toBe(200)
    expect(getAgentHistoryPage).toHaveBeenCalledWith('main', {
      sessionKey: 'session-1',
      cursor: 'abc',
      limit: 25,
    })
    expect(await response.json()).toEqual({
      agentId: 'main',
      sessionKey: 'session-1',
      session: {
        key: 'session-1',
        updatedAt: 20,
        sessionId: 'session-1',
        agentId: 'main',
        kind: 'chat',
        source: 'other',
      },
      items: [
        {
          id: 'session-1:0',
          role: 'user',
          text: 'Hello',
          timestamp: 1,
          messageSeq: 0,
          sessionKey: 'session-1',
          source: 'other',
        },
      ],
      page: {
        cursor: 'older-cursor',
        hasMore: true,
        limit: 25,
      },
    })
  })

  it('ignores role fields when creating agents', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const createAgent = mock(async () => ({
      agentId: 'research',
      name: 'research',
      workspace: '/home/node/.openclaw/workspace-research',
    }))

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () =>
        ({
          createAgent,
        }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/agents', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        name: 'research',
        roleId: 'chief-of-staff',
        customRole: {
          name: 'Ignored',
          shortDescription: 'Ignored',
          longDescription: 'Ignored',
          recommendedApps: [],
          boundaries: [],
        },
        providerType: 'openai',
        apiKey: 'sk-test',
        modelId: 'gpt-5.4-mini',
      }),
    })

    expect(response.status).toBe(201)
    expect(createAgent).toHaveBeenCalledWith({
      name: 'research',
      providerType: 'openai',
      providerName: undefined,
      baseUrl: undefined,
      apiKey: 'sk-test',
      modelId: 'gpt-5.4-mini',
    })
  })

  it('returns JSON history from the session history route and forwards query params', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const getSessionHistory = mock(async () => ({
      sessionKey: 'agent:main:main',
      messages: [{ role: 'user', content: 'hi', messageSeq: 1 }],
      cursor: null,
      hasMore: false,
    }))

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () => ({ getSessionHistory }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request(
      '/session/agent%3Amain%3Amain/history?limit=25&cursor=next',
    )

    expect(response.status).toBe(200)
    expect(response.headers.get('Content-Type')).toContain('application/json')
    expect(getSessionHistory).toHaveBeenCalledWith('agent:main:main', {
      limit: 25,
      cursor: 'next',
    })
    expect(await response.json()).toEqual({
      sessionKey: 'agent:main:main',
      messages: [{ role: 'user', content: 'hi', messageSeq: 1 }],
      cursor: null,
      hasMore: false,
    })
  })

  it('returns 404 when the service reports a missing session', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const getSessionHistory = mock(async () => {
      throw new OpenClawSessionNotFoundError('missing')
    })

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () => ({ getSessionHistory }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/session/missing/history')

    expect(response.status).toBe(404)
    expect(await response.json()).toEqual({
      error: 'OpenClaw session not found: missing',
    })
  })

  it('streams named SSE frames when Accept: text/event-stream', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const streamSessionHistory = mock(
      async () =>
        new ReadableStream({
          start(controller) {
            controller.enqueue({
              type: 'history',
              data: {
                sessionKey: 'k',
                messages: [],
                cursor: null,
                hasMore: false,
              },
            })
            controller.enqueue({
              type: 'message',
              data: {
                sessionKey: 'k',
                messageSeq: 2,
                message: { role: 'assistant', content: 'hi', messageSeq: 2 },
              },
            })
            controller.close()
          },
        }),
    )

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () => ({ streamSessionHistory }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/session/k/history', {
      headers: { Accept: 'text/event-stream' },
    })

    expect(response.status).toBe(200)
    expect(response.headers.get('Content-Type')).toContain('text/event-stream')
    expect(response.headers.get('X-Session-Key')).toBe('k')
    expect(streamSessionHistory).toHaveBeenCalledTimes(1)
    expect(streamSessionHistory.mock.calls[0]?.[0]).toBe('k')
    expect(await response.text()).toBe(
      'event: history\ndata: {"sessionKey":"k","messages":[],"cursor":null,"hasMore":false}\n\n' +
        'event: message\ndata: {"sessionKey":"k","messageSeq":2,"message":{"role":"assistant","content":"hi","messageSeq":2}}\n\n',
    )
  })
})
