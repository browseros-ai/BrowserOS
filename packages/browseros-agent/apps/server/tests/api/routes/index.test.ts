/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it, mock } from 'bun:test'
import { Hono } from 'hono'
import { KlavisService } from '../../../src/api/services/klavis'
import type { Env } from '../../../src/api/types'

mock.module('../../../src/lib/mcp-manager', () => ({
  humaniseInstallError: (err: unknown) => ({
    message: err instanceof Error ? err.message : String(err),
    status: 500,
  }),
  installInto: mock(async () => ({ success: true })),
  listAgents: mock(async () => []),
  uninstallFrom: mock(async () => ({ success: true })),
}))

const { createApiRoutes } = await import('../../../src/api/routes')

function createTestConfig() {
  return {
    port: 32123,
    version: '0.0.0-test',
    browser: {
      isCdpConnected: () => false,
    },
    browserSession: {},
    executionDir: '/tmp/browseros-test',
    resourcesDir: '/tmp/browseros-resources',
    aiSdkDevtoolsEnabled: false,
  } as never
}

function createTestApp(
  agentRoutes = new Hono<Env>(),
  onShutdown: () => void = () => {},
) {
  return createApiRoutes({
    agentRoutes,
    config: createTestConfig(),
    klavis: new KlavisService({ browserosId: null }),
    onShutdown,
    tokenManager: null,
  })
}

const localServer = {
  server: {
    requestIP: () => ({ address: '127.0.0.1' }),
  },
} as never

describe('createApiRoutes', () => {
  it('mounts the canonical system health route', async () => {
    const response = await createTestApp().request('/system/health')

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({
      status: 'ok',
      cdpConnected: false,
    })
  })

  it('keeps the health compatibility route', async () => {
    const response = await createTestApp().request('/health')

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({
      status: 'ok',
      cdpConnected: false,
    })
  })

  it('mounts the canonical system shutdown route', async () => {
    const onShutdown = mock(() => {})
    const response = await createTestApp(undefined, onShutdown).request(
      '/system/shutdown',
      {
        method: 'POST',
      },
    )

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({ status: 'ok' })

    await new Promise<void>((resolve) => setImmediate(resolve))

    expect(onShutdown).toHaveBeenCalledTimes(1)
  })

  it('keeps the shutdown compatibility route', async () => {
    const onShutdown = mock(() => {})
    const response = await createTestApp(undefined, onShutdown).request(
      '/shutdown',
      {
        method: 'POST',
      },
    )

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({ status: 'ok' })

    await new Promise<void>((resolve) => setImmediate(resolve))

    expect(onShutdown).toHaveBeenCalledTimes(1)
  })

  it('preserves the OAuth unavailable fallback', async () => {
    const response = await createTestApp().request('/oauth/openai/status')

    expect(response.status).toBe(503)
    await expect(response.json()).resolves.toEqual({
      error: 'OAuth not available',
    })
  })

  it('mounts the MCP manager routes', async () => {
    const response = await createTestApp().request('/mcp-manager/agents')

    expect(response.status).toBe(200)
    await expect(response.json()).resolves.toEqual({ agents: [] })
  })

  it('keeps injected agent routes behind app-origin auth', async () => {
    const agentRoutes = new Hono<Env>().post('/guard-check', (c) =>
      c.json({ ok: true }),
    )
    const app = createTestApp(agentRoutes)

    const blocked = await app.request('/agents/guard-check', {
      method: 'POST',
    })
    expect(blocked.status).toBe(403)

    const allowed = await app.request(
      '/agents/guard-check',
      {
        method: 'POST',
        headers: {
          Host: 'localhost',
          Origin: 'chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk',
        },
      },
      localServer,
    )
    expect(allowed.status).toBe(200)
    await expect(allowed.json()).resolves.toEqual({ ok: true })
  })

  it('requires a trusted local app request before ACP chat dispatch', async () => {
    const body = JSON.stringify({
      target: {
        type: 'claude',
        agentId: '00000000-0000-4000-8000-000000000001',
      },
      conversationId: '00000000-0000-4000-8000-000000000002',
      message: 'run a command',
    })
    const app = createTestApp()

    const originless = await app.request('/chat', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body,
    })
    expect(originless.status).toBe(403)

    const remote = await app.request(
      '/chat',
      {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Origin: 'chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk',
        },
        body,
      },
      {
        server: { requestIP: () => ({ address: '192.168.1.20' }) },
      } as never,
    )
    expect(remote.status).toBe(403)
  })

  it('requires a trusted local app request before probing host agents', async () => {
    const response = await createTestApp().request('/acpx/probe', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ type: 'claude' }),
    })

    expect(response.status).toBe(403)
  })
})
