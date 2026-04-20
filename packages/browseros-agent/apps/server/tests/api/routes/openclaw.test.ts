/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it, mock } from 'bun:test'
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { UnsupportedOpenClawProviderError } from '../../../src/api/services/openclaw/openclaw-provider-map'

describe('createOpenClawRoutes', () => {
  afterEach(() => {
    mock.restore()
  })

  it('preserves BrowserOS SSE framing, session headers, and defaults chat history for chat', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
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

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/agents/research/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        message: 'hi',
        sessionKey: 'session-123',
      }),
    })

    expect(response.status).toBe(200)
    expect(response.headers.get('Content-Type')).toContain('text/event-stream')
    expect(response.headers.get('X-Session-Key')).toBe('session-123')
    expect(chatStream).toHaveBeenCalledWith('research', 'session-123', 'hi', [])
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
    )
  })

  it('rejects concurrent monitored chat requests for the same agent', async () => {
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
          getActiveSessionId: (agentId: string) =>
            agentId === 'research' ? 'existing-run' : undefined,
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

    expect(response.status).toBe(409)
    expect(chatStream).not.toHaveBeenCalled()
    expect(await response.json()).toEqual({
      error:
        'A monitored chat session is already active for this agent. Wait for it to finish before starting another.',
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

  it('returns the current podman overrides on GET', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const getPodmanOverrides = mock(async () => ({
      podmanPath: '/opt/homebrew/bin/podman',
      effectivePodmanPath: '/opt/homebrew/bin/podman',
    }))

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () => ({ getPodmanOverrides }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/podman-overrides')
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({
      podmanPath: '/opt/homebrew/bin/podman',
      effectivePodmanPath: '/opt/homebrew/bin/podman',
    })
  })

  it('rejects a relative podman path on POST', async () => {
    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/podman-overrides', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ podmanPath: 'podman' }),
    })
    expect(response.status).toBe(400)
    expect(await response.json()).toEqual({
      error: 'podmanPath must be an absolute path',
    })
  })

  it('rejects a nonexistent podman path on POST', async () => {
    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/podman-overrides', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ podmanPath: '/does/not/exist/podman' }),
    })
    expect(response.status).toBe(400)
    expect(await response.json()).toEqual({
      error: 'File does not exist: /does/not/exist/podman',
    })
  })

  it('rejects a non-executable podman path on POST', async () => {
    const tempDir = mkdtempSync(join(tmpdir(), 'openclaw-route-'))
    const nonExec = join(tempDir, 'podman')
    writeFileSync(nonExec, 'not a binary')
    chmodSync(nonExec, 0o644)
    try {
      const { createOpenClawRoutes } = await import(
        '../../../src/api/routes/openclaw'
      )
      const route = createOpenClawRoutes()

      const response = await route.request('/podman-overrides', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ podmanPath: nonExec }),
      })
      expect(response.status).toBe(400)
      expect(await response.json()).toEqual({
        error: `File is not executable: ${nonExec}`,
      })
    } finally {
      rmSync(tempDir, { recursive: true, force: true })
    }
  })

  it('applies and echoes when POST clears the override', async () => {
    const actualOpenClawService = await import(
      '../../../src/api/services/openclaw/openclaw-service'
    )
    const applyPodmanOverrides = mock(async () => ({
      podmanPath: null,
      effectivePodmanPath: 'podman',
    }))

    mock.module('../../../src/api/services/openclaw/openclaw-service', () => ({
      ...actualOpenClawService,
      getOpenClawService: () => ({ applyPodmanOverrides }) as never,
    }))

    const { createOpenClawRoutes } = await import(
      '../../../src/api/routes/openclaw'
    )
    const route = createOpenClawRoutes()

    const response = await route.request('/podman-overrides', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ podmanPath: null }),
    })
    expect(response.status).toBe(200)
    expect(applyPodmanOverrides).toHaveBeenCalledWith({ podmanPath: null })
    expect(await response.json()).toEqual({
      podmanPath: null,
      effectivePodmanPath: 'podman',
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
})
