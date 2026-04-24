/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it, mock } from 'bun:test'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { OPENCLAW_CONTAINER_HOME } from '@browseros/shared/constants/openclaw'
import {
  resolveSupportedOpenClawProvider,
  UnsupportedOpenClawProviderError,
} from '../../../../src/api/services/openclaw/openclaw-provider-map'
import {
  normalizeBrowserOSChatSessionKey,
  OpenClawService,
} from '../../../../src/api/services/openclaw/openclaw-service'

type MutableOpenClawService = OpenClawService & {
  openclawDir: string
  token: string
  restart: ReturnType<typeof mock>
  runtime: {
    ensureReady?: () => Promise<void>
    isPodmanAvailable?: () => Promise<boolean>
    getMachineStatus?: () => Promise<{ initialized: boolean; running: boolean }>
    isHealthy?: (_hostPort?: number) => Promise<boolean>
    isReady: (_hostPort?: number) => Promise<boolean>
    pullImage?: (
      _image: string,
      _onLog?: (_line: string) => void,
    ) => Promise<void>
    startGateway?: (
      _input: unknown,
      _onLog?: (_line: string) => void,
    ) => Promise<void>
    restartGateway?: (
      _input: unknown,
      _onLog?: (_line: string) => void,
    ) => Promise<void>
    stopGateway?: (_onLog?: (_line: string) => void) => Promise<void>
    getGatewayLogs?: (_tail?: number) => Promise<string[]>
    waitForReady?: () => Promise<boolean>
    stopVm?: () => Promise<void>
  }
  cliClient: {
    probe?: ReturnType<typeof mock>
    createAgent?: ReturnType<typeof mock>
    getConfig?: ReturnType<typeof mock>
    getChatHistory?: ReturnType<typeof mock>
    listAgents?: ReturnType<typeof mock>
    listSessions?: ReturnType<typeof mock>
    setDefaultModel?: ReturnType<typeof mock>
  }
  httpClient: {
    streamChat?: ReturnType<typeof mock>
    getSessionHistory?: ReturnType<typeof mock>
  }
  bootstrapCliClient: {
    runOnboard?: ReturnType<typeof mock>
    setConfigBatch?: ReturnType<typeof mock>
    setDefaultModel?: ReturnType<typeof mock>
    validateConfig?: ReturnType<typeof mock>
  }
}

describe('OpenClawService', () => {
  let tempDir: string | null = null
  const originalFetch = globalThis.fetch

  afterEach(async () => {
    mock.restore()
    globalThis.fetch = originalFetch
    if (tempDir) {
      await rm(tempDir, { recursive: true, force: true })
      tempDir = null
    }
  })

  it('creates agents through the cli client without role bootstrap files', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    const createAgent = mock(async () => ({
      agentId: 'ops',
      name: 'ops',
      workspace: `${OPENCLAW_CONTAINER_HOME}/workspace-ops`,
      model: 'openclaw/default',
    }))
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      isReady: async () => true,
    }
    service.cliClient = {
      createAgent,
    }

    const agent = await service.createAgent({
      name: 'ops',
    })

    expect(createAgent).toHaveBeenCalledWith({
      name: 'ops',
      model: undefined,
    })
    expect(agent).toEqual({
      agentId: 'ops',
      name: 'ops',
      workspace: `${OPENCLAW_CONTAINER_HOME}/workspace-ops`,
      model: 'openclaw/default',
    })
    expect(
      existsSync(
        join(tempDir, '.openclaw', 'workspace-ops', '.browseros-role.json'),
      ),
    ).toBe(false)
  })

  it('lists plain agent entries without role metadata', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw', 'workspace-ops'), {
      recursive: true,
    })
    await writeFile(join(tempDir, '.openclaw', 'openclaw.json'), '{}')
    await writeFile(
      join(tempDir, '.openclaw', 'workspace-ops', '.browseros-role.json'),
      '{"roleId":"chief-of-staff"}\n',
      'utf-8',
    )
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      isReady: async () => true,
    }
    service.cliClient = {
      getConfig: mock(async () => 'cli-token'),
      listAgents: mock(async () => [
        {
          agentId: 'ops',
          name: 'ops',
          workspace: `${OPENCLAW_CONTAINER_HOME}/workspace-ops`,
          model: 'openai/gpt-5.4-mini',
        },
      ]),
    }

    await expect(service.listAgents()).resolves.toEqual([
      {
        agentId: 'ops',
        name: 'ops',
        workspace: `${OPENCLAW_CONTAINER_HOME}/workspace-ops`,
        model: 'openai/gpt-5.4-mini',
      },
    ])
  })

  it('resolves the latest user-chat session for an agent', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw', 'agents', 'main', 'sessions'), {
      recursive: true,
    })
    await writeFile(
      join(tempDir, '.openclaw', 'agents', 'main', 'sessions', 'sessions.json'),
      JSON.stringify({
        'agent:main:cron:daily': {
          sessionId: 'cron-session',
          updatedAt: 30,
        },
        'openai-user:browseros:main:chat-session': {
          sessionId: 'chat-session',
          updatedAt: 20,
        },
      }),
    )
    const service = new OpenClawService() as MutableOpenClawService
    service.openclawDir = tempDir

    expect(service.resolveAgentSession('main')).toEqual({
      agentId: 'main',
      exists: true,
      sessionKey: 'openai-user:browseros:main:chat-session',
      session: {
        key: 'openai-user:browseros:main:chat-session',
        updatedAt: 20,
        sessionId: 'chat-session',
        agentId: 'main',
        kind: 'chat',
        source: 'user-chat',
      },
    })
  })

  it('normalizes recursive OpenClaw BrowserOS session keys to the raw chat session id', () => {
    expect(
      normalizeBrowserOSChatSessionKey(
        'main',
        'agent:main:openai-user:browseros:main:e1ee8e17-4fdb-4072-99ce-8f680853ec00',
      ),
    ).toBe('e1ee8e17-4fdb-4072-99ce-8f680853ec00')
    expect(
      normalizeBrowserOSChatSessionKey(
        'main',
        'agent:main:openai-user:browseros:main:agent:main:openai-user:browseros:main:e1ee8e17-4fdb-4072-99ce-8f680853ec00',
      ),
    ).toBe('e1ee8e17-4fdb-4072-99ce-8f680853ec00')
  })

  it('returns the raw BrowserOS session id while retaining the OpenClaw key for diagnostics', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw', 'agents', 'main', 'sessions'), {
      recursive: true,
    })
    await writeFile(
      join(tempDir, '.openclaw', 'agents', 'main', 'sessions', 'sessions.json'),
      JSON.stringify({
        'agent:main:openai-user:browseros:main:e1ee8e17-4fdb-4072-99ce-8f680853ec00':
          {
            sessionId: 'chat-session',
            updatedAt: 20,
          },
      }),
    )
    const service = new OpenClawService() as MutableOpenClawService
    service.openclawDir = tempDir

    expect(service.resolveAgentSession('main')).toEqual({
      agentId: 'main',
      exists: true,
      sessionKey: 'e1ee8e17-4fdb-4072-99ce-8f680853ec00',
      session: {
        key: 'agent:main:openai-user:browseros:main:e1ee8e17-4fdb-4072-99ce-8f680853ec00',
        updatedAt: 20,
        sessionId: 'chat-session',
        agentId: 'main',
        kind: 'chat',
        source: 'user-chat',
      },
    })
  })

  it('resolves recursive active sessions back to the canonical OpenClaw transcript key', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw', 'agents', 'main', 'sessions'), {
      recursive: true,
    })
    await writeFile(
      join(tempDir, '.openclaw', 'agents', 'main', 'sessions', 'sessions.json'),
      JSON.stringify({
        'agent:main:openai-user:browseros:main:agent:main:openai-user:browseros:main:e1ee8e17-4fdb-4072-99ce-8f680853ec00':
          {
            sessionId: 'nested-session',
            updatedAt: 30,
          },
        'agent:main:openai-user:browseros:main:e1ee8e17-4fdb-4072-99ce-8f680853ec00':
          {
            sessionId: 'canonical-session',
            updatedAt: 20,
          },
      }),
    )
    const service = new OpenClawService() as MutableOpenClawService
    service.openclawDir = tempDir

    expect(service.resolveAgentSession('main')).toEqual({
      agentId: 'main',
      exists: true,
      sessionKey: 'e1ee8e17-4fdb-4072-99ce-8f680853ec00',
      session: {
        key: 'agent:main:openai-user:browseros:main:e1ee8e17-4fdb-4072-99ce-8f680853ec00',
        updatedAt: 20,
        sessionId: 'canonical-session',
        agentId: 'main',
        kind: 'chat',
        source: 'user-chat',
      },
    })
  })

  it('uses the canonical OpenClaw key when history is requested with a recursive session key', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw', 'agents', 'main', 'sessions'), {
      recursive: true,
    })
    await writeFile(
      join(tempDir, '.openclaw', 'agents', 'main', 'sessions', 'sessions.json'),
      JSON.stringify({
        'agent:main:openai-user:browseros:main:e1ee8e17-4fdb-4072-99ce-8f680853ec00':
          {
            sessionId: 'chat-session',
            updatedAt: 20,
          },
      }),
    )
    await writeFile(
      join(
        tempDir,
        '.openclaw',
        'agents',
        'main',
        'sessions',
        'chat-session.jsonl',
      ),
      [
        '{"type":"message","id":"m1","timestamp":"1970-01-01T00:00:00.001Z","message":{"role":"user","content":[{"type":"text","text":"Old question"}]}}',
        '{"type":"message","id":"m2","timestamp":"1970-01-01T00:00:00.002Z","message":{"role":"assistant","content":[{"type":"text","text":"Old answer"}]}}',
      ].join('\n'),
    )
    const service = new OpenClawService() as MutableOpenClawService
    service.openclawDir = tempDir

    const page = service.getAgentHistoryPage('main', {
      sessionKey:
        'agent:main:openai-user:browseros:main:agent:main:openai-user:browseros:main:e1ee8e17-4fdb-4072-99ce-8f680853ec00',
    })

    expect(page.sessionKey).toBe('e1ee8e17-4fdb-4072-99ce-8f680853ec00')
    expect(page.items).toEqual([
      {
        id: 'e1ee8e17-4fdb-4072-99ce-8f680853ec00:0',
        role: 'user',
        text: 'Old question',
        timestamp: 1,
        messageSeq: 0,
        sessionKey: 'e1ee8e17-4fdb-4072-99ce-8f680853ec00',
        source: 'user-chat',
      },
      {
        id: 'e1ee8e17-4fdb-4072-99ce-8f680853ec00:1',
        role: 'assistant',
        text: 'Old answer',
        timestamp: 2,
        messageSeq: 1,
        sessionKey: 'e1ee8e17-4fdb-4072-99ce-8f680853ec00',
        source: 'user-chat',
      },
    ])
  })

  it('returns normalized paginated chat history for an agent session', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw', 'agents', 'main', 'sessions'), {
      recursive: true,
    })
    await writeFile(
      join(tempDir, '.openclaw', 'agents', 'main', 'sessions', 'sessions.json'),
      JSON.stringify({
        'openai-user:browseros:main:chat-session': {
          sessionId: 'pi-session',
          updatedAt: 20,
        },
      }),
    )
    await writeFile(
      join(
        tempDir,
        '.openclaw',
        'agents',
        'main',
        'sessions',
        'pi-session.jsonl',
      ),
      [
        '{"type":"message","id":"m0","timestamp":"1970-01-01T00:00:00.000Z","message":{"role":"assistant","content":[{"type":"text","text":"HEARTBEAT_OK"}]}}',
        '{"type":"message","id":"m1","timestamp":"1970-01-01T00:00:00.001Z","message":{"role":"user","content":[{"type":"text","text":"First question"}]}}',
        '{"type":"message","id":"m2","timestamp":"1970-01-01T00:00:00.002Z","message":{"role":"assistant","content":[{"type":"text","text":"First answer"}]}}',
        '{"type":"message","id":"m3","timestamp":"1970-01-01T00:00:00.003Z","message":{"role":"user","content":[{"type":"text","text":"[Chat messages since your last reply]\\n[Current message - respond to this]\\nUser: Second question"}]}}',
      ].join('\n'),
    )
    const service = new OpenClawService() as MutableOpenClawService
    service.openclawDir = tempDir

    const page = service.getAgentHistoryPage('main', { limit: 2 })

    expect(page.agentId).toBe('main')
    expect(page.sessionKey).toBe('openai-user:browseros:main:chat-session')
    expect(page.items).toEqual([
      {
        id: 'openai-user:browseros:main:chat-session:1',
        role: 'assistant',
        text: 'First answer',
        timestamp: 2,
        messageSeq: 1,
        sessionKey: 'openai-user:browseros:main:chat-session',
        source: 'user-chat',
      },
      {
        id: 'openai-user:browseros:main:chat-session:2',
        role: 'user',
        text: 'Second question',
        timestamp: 3,
        messageSeq: 2,
        sessionKey: 'openai-user:browseros:main:chat-session',
        source: 'user-chat',
      },
    ])
    expect(page.page.hasMore).toBe(true)
    expect(typeof page.page.cursor).toBe('string')
  })

  it('normalizes recursive session keys before streaming chat', async () => {
    const service = new OpenClawService() as MutableOpenClawService
    const stream = new ReadableStream()
    const streamChat = mock(async () => stream)

    service.runtime = {
      isReady: async () => true,
    }
    service.httpClient = {
      streamChat,
    }

    await expect(
      service.chatStream(
        'main',
        'agent:main:openai-user:browseros:main:agent:main:openai-user:browseros:main:e1ee8e17-4fdb-4072-99ce-8f680853ec00',
        'hello',
      ),
    ).resolves.toBe(stream)
    expect(streamChat).toHaveBeenCalledWith({
      agentId: 'main',
      sessionKey: 'e1ee8e17-4fdb-4072-99ce-8f680853ec00',
      message: 'hello',
      history: [],
    })
  })

  it('maps successful cli client probes into connected status', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(join(tempDir, '.openclaw', 'openclaw.json'), '{}')
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      isPodmanAvailable: async () => true,
      getMachineStatus: async () => ({ initialized: true, running: true }),
      isReady: async () => true,
    }
    service.cliClient = {
      getConfig: mock(async () => 'cli-token'),
      listAgents: mock(async () => [
        {
          agentId: 'main',
          name: 'main',
          workspace: `${OPENCLAW_CONTAINER_HOME}/workspace`,
        },
        {
          agentId: 'ops',
          name: 'ops',
          workspace: `${OPENCLAW_CONTAINER_HOME}/workspace-ops`,
        },
      ]),
    }

    const status = await service.getStatus()

    expect(status).toEqual({
      status: 'running',
      podmanAvailable: true,
      machineReady: true,
      port: 18789,
      agentCount: 2,
      error: null,
      controlPlaneStatus: 'connected',
      lastGatewayError: null,
      lastRecoveryReason: null,
    })
  })

  it('creates the main agent during setup when the gateway starts without one', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    const steps: string[] = []
    const runOnboard = mock(async () => {
      steps.push('onboard')
    })
    const setConfigBatch = mock(async () => {
      steps.push('batch')
    })
    const setDefaultModel = mock(async () => {})
    const validateConfig = mock(async () => {
      steps.push('validate')
      return { ok: true }
    })
    const createAgent = mock(async () => ({
      agentId: 'main',
      name: 'main',
      workspace: `${OPENCLAW_CONTAINER_HOME}/workspace`,
    }))
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    const restartGateway = mock(async () => {
      steps.push('restart')
    })
    const startGateway = mock(async () => {
      steps.push('start')
    })
    service.runtime = {
      isPodmanAvailable: async () => true,
      ensureReady: async () => {},
      isReady: async () => true,
      restartGateway,
      startGateway,
      waitForReady: mock(async () => {
        steps.push('ready')
        return true
      }),
    }
    service.cliClient = {
      probe: mock(async () => {}),
      listAgents: mock(async () => []),
      createAgent,
    }
    service.bootstrapCliClient = {
      runOnboard,
      setConfigBatch,
      setDefaultModel,
      validateConfig,
    }

    await service.setup({})

    expect(runOnboard).toHaveBeenCalledWith({
      acceptRisk: true,
      authChoice: 'skip',
      gatewayAuth: 'token',
      gatewayBind: 'lan',
      gatewayPort: 18789,
      installDaemon: false,
      mode: 'local',
      nonInteractive: true,
      skipHealth: true,
    })
    expect(setConfigBatch).toHaveBeenCalledWith(
      expect.arrayContaining([
        {
          path: 'mcp.servers.browseros.url',
          value: 'http://host.containers.internal:9100/mcp',
        },
        {
          path: 'mcp.servers.browseros.transport',
          value: 'streamable-http',
        },
        {
          path: 'gateway.http.endpoints.chatCompletions.enabled',
          value: true,
        },
      ]),
    )
    expect(validateConfig).toHaveBeenCalled()
    expect(createAgent).toHaveBeenCalledWith({
      name: 'main',
      model: undefined,
    })
    expect(steps).toEqual(['onboard', 'batch', 'validate', 'start', 'ready'])
    expect(startGateway).toHaveBeenCalledWith(
      expect.objectContaining({
        image: 'ghcr.io/openclaw/openclaw:2026.4.12',
        hostPort: expect.any(Number),
        hostHome: tempDir,
        envFilePath: join(tempDir, '.openclaw', '.env'),
        gatewayToken: undefined,
      }),
      expect.any(Function),
    )
    expect(restartGateway).not.toHaveBeenCalled()
  })

  it('applies setup-time config in one batch before the gateway starts', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    const runOnboard = mock(async () => {})
    const setConfigBatch = mock(async () => {})
    const validateConfig = mock(async () => ({ ok: true }))
    const createAgent = mock(async () => ({
      agentId: 'main',
      name: 'main',
      workspace: `${OPENCLAW_CONTAINER_HOME}/workspace`,
    }))
    const waitForReady = mock(async () => true)
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    const restartGateway = mock(async () => {})
    const startGateway = mock(async () => {})
    service.runtime = {
      isPodmanAvailable: async () => true,
      ensureReady: async () => {},
      isReady: async () => true,
      pullImage: async () => {},
      restartGateway,
      startGateway,
      waitForReady,
    }
    service.cliClient = {
      probe: mock(async () => {}),
      listAgents: mock(async () => []),
      createAgent,
    }
    service.bootstrapCliClient = {
      runOnboard,
      setConfigBatch,
      setDefaultModel: mock(async () => {}),
      validateConfig,
    }

    await expect(service.setup({})).resolves.toBeUndefined()

    expect(setConfigBatch).toHaveBeenCalledTimes(1)
    expect(waitForReady).toHaveBeenCalledTimes(1)
    expect(createAgent).toHaveBeenCalledWith({
      name: 'main',
      model: undefined,
    })
    expect(startGateway).toHaveBeenCalledTimes(1)
    expect(restartGateway).not.toHaveBeenCalled()
  })

  it('loads the persisted gateway token from the mounted config before control plane calls', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
      }),
    )
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.token = 'random-token'
    service.runtime = {
      isReady: async () => true,
    }
    service.cliClient = {
      listAgents: mock(async () => {
        expect(service.token).toBe('cli-token')
        return []
      }),
    }

    await service.listAgents()
  })

  it('caches the loaded gateway token from config across steady-state control plane calls', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
      }),
    )
    const listAgents = mock(async () => [])
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      isReady: async () => true,
    }
    service.cliClient = {
      listAgents,
    }

    await service.listAgents()
    await service.listAgents()

    expect(listAgents).toHaveBeenCalledTimes(2)
  })

  it('writes provider credentials into the mounted state env file during setup', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      isPodmanAvailable: async () => true,
      ensureReady: async () => {},
      isReady: async () => true,
      pullImage: async () => {},
      restartGateway: async () => {},
      startGateway: async () => {},
      waitForReady: async () => true,
    }
    service.cliClient = {
      getConfig: mock(async (path: string) =>
        path === 'gateway.auth.token' ? 'cli-token' : null,
      ),
      probe: mock(async () => {}),
      listAgents: mock(async () => [
        {
          agentId: 'main',
          name: 'main',
          workspace: `${OPENCLAW_CONTAINER_HOME}/workspace`,
        },
      ]),
      createAgent: mock(async () => {
        throw new Error('createAgent should not be called when main exists')
      }),
    }
    service.bootstrapCliClient = {
      runOnboard: mock(async () => {}),
      setConfigBatch: mock(async () => {}),
      setDefaultModel: mock(async () => {}),
      validateConfig: mock(async () => ({ ok: true })),
    }

    await service.setup({
      providerType: 'openai',
      apiKey: 'sk-test',
      modelId: 'gpt-5.4-mini',
    })

    expect(
      await readFile(join(tempDir, '.openclaw', '.env'), 'utf-8'),
    ).toContain('OPENAI_API_KEY=sk-test')
  })

  it('merges custom openai-compatible providers into config during setup', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    const openclawTempDir = tempDir
    const runOnboard = mock(async () => {
      await mkdir(join(openclawTempDir, '.openclaw'), { recursive: true })
      await writeFile(
        join(openclawTempDir, '.openclaw', 'openclaw.json'),
        JSON.stringify({
          gateway: {
            auth: {
              token: 'cli-token',
            },
          },
        }),
        'utf-8',
      )
    })
    const setConfigBatch = mock(async () => {})
    const setDefaultModel = mock(async () => {})
    const validateConfig = mock(async () => ({ ok: true }))
    const createAgent = mock(async () => ({
      agentId: 'main',
      name: 'main',
      workspace: `${OPENCLAW_CONTAINER_HOME}/workspace`,
    }))
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      isPodmanAvailable: async () => true,
      ensureReady: async () => {},
      isReady: async () => true,
      pullImage: async () => {},
      restartGateway: async () => {},
      startGateway: async () => {},
      waitForReady: async () => true,
    }
    service.cliClient = {
      probe: mock(async () => {}),
      listAgents: mock(async () => []),
      createAgent,
    }
    service.bootstrapCliClient = {
      runOnboard,
      setConfigBatch,
      setDefaultModel,
      validateConfig,
    }

    await service.setup({
      providerType: 'openai-compatible',
      providerName: 'Kimi K2.5',
      baseUrl: 'https://api.fireworks.ai/inference/v1',
      apiKey: 'custom-key',
      modelId: 'accounts/fireworks/models/kimi-k2p5',
    })

    expect(setDefaultModel).toHaveBeenCalledWith(
      'kimi-k2-5/accounts/fireworks/models/kimi-k2p5',
    )
    expect(
      await readFile(join(tempDir, '.openclaw', '.env'), 'utf-8'),
    ).toContain('KIMI_K2_5_API_KEY=custom-key')
    expect(
      JSON.parse(
        await readFile(join(tempDir, '.openclaw', 'openclaw.json'), 'utf-8'),
      ),
    ).toMatchObject({
      gateway: {
        auth: {
          token: 'cli-token',
        },
      },
      models: {
        mode: 'merge',
        providers: {
          'kimi-k2-5': {
            api: 'openai-completions',
            baseUrl: 'https://api.fireworks.ai/inference/v1',
            apiKey: `\${KIMI_K2_5_API_KEY}`,
            models: [
              {
                id: 'accounts/fireworks/models/kimi-k2p5',
                name: 'accounts/fireworks/models/kimi-k2p5',
              },
            ],
          },
        },
      },
    })
  })

  it('start uses the direct runtime startGateway flow', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
      }),
    )
    const ensureReady = mock(async () => {})
    const startGateway = mock(async () => {})
    const waitForReady = mock(async () => true)
    const probe = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      ensureReady,
      isReady: async () => false,
      startGateway,
      waitForReady,
    }
    service.cliClient = {
      probe,
    }

    await service.start()

    expect(ensureReady).toHaveBeenCalledTimes(1)
    expect(startGateway).toHaveBeenCalledWith(
      expect.objectContaining({
        image: 'ghcr.io/openclaw/openclaw:2026.4.12',
        hostPort: expect.any(Number),
        hostHome: tempDir,
        envFilePath: join(tempDir, '.openclaw', '.env'),
        gatewayToken: 'cli-token',
      }),
      expect.any(Function),
    )
    expect(waitForReady).toHaveBeenCalledTimes(1)
    expect(probe).toHaveBeenCalledTimes(1)
  })

  it('serializes concurrent start calls and only starts the gateway once', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
      }),
    )
    let gatewayReady = false
    let releaseStartGateway!: () => void
    let notifyStartGatewayEntered!: () => void
    const startGatewayEntered = new Promise<void>((resolve) => {
      notifyStartGatewayEntered = resolve
    })
    const unblockStartGateway = new Promise<void>((resolve) => {
      releaseStartGateway = resolve
    })
    const ensureReady = mock(async () => {})
    const startGateway = mock(async () => {
      notifyStartGatewayEntered()
      await unblockStartGateway
      gatewayReady = true
    })
    const waitForReady = mock(async () => true)
    const probe = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      ensureReady,
      isReady: async () => gatewayReady,
      startGateway,
      waitForReady,
    }
    service.cliClient = {
      probe,
    }
    mockGatewayAuth()

    const firstStart = service.start()
    await startGatewayEntered
    const secondStart = service.start()
    releaseStartGateway()
    await Promise.all([firstStart, secondStart])

    expect(ensureReady).toHaveBeenCalledTimes(2)
    expect(startGateway).toHaveBeenCalledTimes(1)
    expect(waitForReady).toHaveBeenCalledTimes(1)
    expect(probe).toHaveBeenCalledTimes(2)
  })

  it('does not restart a ready gateway when start is called again', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
      }),
    )
    const ensureReady = mock(async () => {})
    const startGateway = mock(async () => {})
    const waitForReady = mock(async () => true)
    const probe = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      ensureReady,
      isReady: async () => true,
      startGateway,
      waitForReady,
    }
    service.cliClient = {
      probe,
    }
    mockGatewayAuth()

    await service.start()

    expect(ensureReady).toHaveBeenCalledTimes(1)
    expect(startGateway).not.toHaveBeenCalled()
    expect(waitForReady).not.toHaveBeenCalled()
    expect(probe).toHaveBeenCalledTimes(1)
  })

  it('restart uses the direct runtime restartGateway flow', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
      }),
    )
    const ensureReady = mock(async () => {})
    const restartGateway = mock(async () => {})
    const waitForReady = mock(async () => true)
    const probe = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      ensureReady,
      isReady: async () => true,
      restartGateway,
      waitForReady,
    }
    service.cliClient = {
      probe,
    }
    mockGatewayAuth()

    await service.restart()

    expect(ensureReady).toHaveBeenCalledTimes(1)
    expect(restartGateway).toHaveBeenCalledWith(
      expect.objectContaining({
        image: 'ghcr.io/openclaw/openclaw:2026.4.12',
        hostPort: expect.any(Number),
        hostHome: tempDir,
        envFilePath: join(tempDir, '.openclaw', '.env'),
        gatewayToken: 'cli-token',
      }),
      expect.any(Function),
    )
    expect(waitForReady).toHaveBeenCalledTimes(1)
    expect(probe).toHaveBeenCalledTimes(1)
  })

  it('restart keeps the persisted gateway port when the current gateway already owns it', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
      }),
    )
    const occupiedServer = createServer()
    const occupiedPort = await new Promise<number>((resolve, reject) => {
      occupiedServer.once('error', reject)
      occupiedServer.listen(0, '127.0.0.1', () => {
        const address = occupiedServer.address()
        if (!address || typeof address === 'string') {
          reject(new Error('failed to allocate test port'))
          return
        }
        resolve(address.port)
      })
    })
    await writeFile(
      join(tempDir, '.openclaw', 'runtime-state.json'),
      `${JSON.stringify({ gatewayPort: occupiedPort }, null, 2)}\n`,
    )
    const ensureReady = mock(async () => {})
    const restartGateway = mock(async () => {})
    const waitForReady = mock(async () => true)
    const probe = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      ensureReady,
      isReady: async (hostPort?: number) => hostPort === occupiedPort,
      restartGateway,
      waitForReady,
    }
    service.cliClient = {
      probe,
    }
    mockGatewayAuth()

    try {
      await service.restart()
    } finally {
      await new Promise<void>((resolve, reject) => {
        occupiedServer.close((error) => {
          if (error) {
            reject(error)
            return
          }
          resolve()
        })
      })
    }

    expect(restartGateway).toHaveBeenCalledWith(
      expect.objectContaining({
        hostPort: occupiedPort,
      }),
      expect.any(Function),
    )
    expect(ensureReady).toHaveBeenCalledTimes(1)
  })

  it('restart moves off a persisted ready port when auth rejects the current token', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
      }),
    )
    const occupiedServer = createServer()
    const occupiedPort = await new Promise<number>((resolve, reject) => {
      occupiedServer.once('error', reject)
      occupiedServer.listen(0, '127.0.0.1', () => {
        const address = occupiedServer.address()
        if (!address || typeof address === 'string') {
          reject(new Error('failed to allocate test port'))
          return
        }
        resolve(address.port)
      })
    })
    await writeFile(
      join(tempDir, '.openclaw', 'runtime-state.json'),
      `${JSON.stringify({ gatewayPort: occupiedPort }, null, 2)}\n`,
    )
    const ensureReady = mock(async () => {})
    const restartGateway = mock(async () => {})
    const waitForReady = mock(async () => true)
    const probe = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      ensureReady,
      isReady: async (hostPort?: number) => hostPort === occupiedPort,
      restartGateway,
      waitForReady,
    }
    service.cliClient = {
      probe,
    }
    mockGatewayAuth(401)

    try {
      await service.restart()
    } finally {
      await new Promise<void>((resolve, reject) => {
        occupiedServer.close((error) => {
          if (error) {
            reject(error)
            return
          }
          resolve()
        })
      })
    }

    expect(restartGateway).toHaveBeenCalledWith(
      expect.objectContaining({
        hostPort: expect.any(Number),
      }),
      expect.any(Function),
    )
    expect(
      (restartGateway.mock.calls[0]?.[0] as { hostPort: number }).hostPort,
    ).not.toBe(occupiedPort)
    expect(ensureReady).toHaveBeenCalledTimes(1)
  })

  it('stop calls runtime.stopGateway', async () => {
    const stopGateway = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.runtime = {
      isReady: async () => true,
      stopGateway,
    }

    await service.stop()

    expect(stopGateway).toHaveBeenCalledTimes(1)
  })

  it('getLogs proxies to runtime.getGatewayLogs with tail', async () => {
    const getGatewayLogs = mock(async (tail = 50) => [`tail:${tail}`])
    const service = new OpenClawService() as MutableOpenClawService

    service.runtime = {
      isReady: async () => true,
      getGatewayLogs,
    }

    await expect(service.getLogs(25)).resolves.toEqual(['tail:25'])
    expect(getGatewayLogs).toHaveBeenCalledWith(25)
  })

  it('shutdown stops gateway and then stops the VM', async () => {
    const stopGateway = mock(async () => {})
    const stopVm = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.runtime = {
      isReady: async () => true,
      stopGateway,
      stopVm,
    }

    await service.shutdown()

    expect(stopGateway).toHaveBeenCalledTimes(1)
    expect(stopVm).toHaveBeenCalledTimes(1)
  })

  it('shutdown still stops the VM when stopGateway fails', async () => {
    const stopGateway = mock(async () => {
      throw new Error('stop failed')
    })
    const stopVm = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.runtime = {
      isReady: async () => true,
      stopGateway,
      stopVm,
    }

    await expect(service.shutdown()).resolves.toBeUndefined()

    expect(stopGateway).toHaveBeenCalledTimes(1)
    expect(stopVm).toHaveBeenCalledTimes(1)
  })

  it('tryAutoStart uses direct-runtime startGateway when gateway is not ready', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
      }),
    )
    const ensureReady = mock(async () => {})
    const isReady = mock(async () => false)
    const startGateway = mock(async () => {})
    const waitForReady = mock(async () => true)
    const probe = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.runtime = {
      isPodmanAvailable: async () => true,
      ensureReady,
      isReady,
      startGateway,
      waitForReady,
    }
    service.cliClient = {
      probe,
    }

    await service.tryAutoStart()

    expect(ensureReady).toHaveBeenCalledTimes(1)
    expect(startGateway).toHaveBeenCalledWith(
      expect.objectContaining({
        image: 'ghcr.io/openclaw/openclaw:2026.4.12',
        hostPort: expect.any(Number),
        hostHome: tempDir,
        envFilePath: join(tempDir, '.openclaw', '.env'),
        gatewayToken: 'cli-token',
      }),
    )
    expect(waitForReady).toHaveBeenCalledTimes(1)
    expect(probe).toHaveBeenCalledTimes(1)
    expect(isReady).toHaveBeenCalledTimes(2)
  })

  it('keeps openrouter model refs verbatim without rewriting dots', () => {
    const provider = resolveSupportedOpenClawProvider({
      providerType: 'openrouter',
      apiKey: 'or-key',
      modelId: 'anthropic/claude-haiku-4.5',
    })

    expect(provider).toEqual({
      envValues: {
        OPENROUTER_API_KEY: 'or-key',
      },
      model: 'openrouter/anthropic/claude-haiku-4.5',
      providerType: 'openrouter',
    })
  })

  it('resolves builtin and custom providers into OpenClaw config inputs', () => {
    expect(
      resolveSupportedOpenClawProvider({
        providerType: 'anthropic',
        apiKey: 'ant-key',
        modelId: 'claude-sonnet-4.5',
      }),
    ).toEqual({
      envValues: {
        ANTHROPIC_API_KEY: 'ant-key',
      },
      model: 'anthropic/claude-sonnet-4.5',
      providerType: 'anthropic',
    })

    expect(
      resolveSupportedOpenClawProvider({
        providerType: 'moonshot',
        apiKey: 'moon-key',
        modelId: 'kimi-k2',
      }),
    ).toEqual({
      envValues: {
        MOONSHOT_API_KEY: 'moon-key',
      },
      model: 'moonshot/kimi-k2',
      providerType: 'moonshot',
    })

    expect(
      resolveSupportedOpenClawProvider({
        providerType: 'openai-compatible',
        providerName: 'Kimi K2.5',
        baseUrl: 'https://api.fireworks.ai/inference/v1',
        apiKey: 'custom-key',
        modelId: 'accounts/fireworks/models/kimi-k2p5',
      }),
    ).toEqual({
      envValues: {
        KIMI_K2_5_API_KEY: 'custom-key',
      },
      model: 'kimi-k2-5/accounts/fireworks/models/kimi-k2p5',
      customProvider: {
        providerId: 'kimi-k2-5',
        apiKeyEnvVar: 'KIMI_K2_5_API_KEY',
        config: {
          api: 'openai-completions',
          baseUrl: 'https://api.fireworks.ai/inference/v1',
          apiKey: `\${KIMI_K2_5_API_KEY}`,
          models: [
            {
              id: 'accounts/fireworks/models/kimi-k2p5',
              name: 'accounts/fireworks/models/kimi-k2p5',
            },
          ],
        },
      },
    })

    expect(() =>
      resolveSupportedOpenClawProvider({
        providerType: 'google',
        apiKey: 'google-key',
        modelId: 'gemini-2.5-pro',
      }),
    ).toThrow(new UnsupportedOpenClawProviderError('google'))
  })

  it('rejects unsupported providers before mutating env or creating agents', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    const createAgent = mock(async () => ({
      agentId: 'ops',
      name: 'ops',
      workspace: `${OPENCLAW_CONTAINER_HOME}/workspace-ops`,
    }))
    const restart = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.restart = restart
    service.runtime = {
      isReady: async () => true,
    }
    service.cliClient = {
      createAgent,
    }

    await expect(
      service.createAgent({
        name: 'ops',
        providerType: 'google',
        apiKey: 'google-key',
        modelId: 'gemini-2.5-pro',
      }),
    ).rejects.toThrow('Unsupported OpenClaw provider')

    expect(createAgent).not.toHaveBeenCalled()
    expect(restart).not.toHaveBeenCalled()
    expect(existsSync(join(tempDir, '.openclaw', '.env'))).toBe(false)
  })

  it('passes openrouter model refs through verbatim into agent creation', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', '.env'),
      'OPENROUTER_API_KEY=or-key\n',
      'utf-8',
    )
    const createAgent = mock(async () => ({
      agentId: 'research',
      name: 'research',
      workspace: `${OPENCLAW_CONTAINER_HOME}/workspace-research`,
      model: 'openrouter/anthropic/claude-haiku-4.5',
    }))
    const restart = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.restart = restart
    service.runtime = {
      isReady: async () => true,
    }
    service.cliClient = {
      createAgent,
    }

    await service.createAgent({
      name: 'research',
      providerType: 'openrouter',
      apiKey: 'or-key',
      modelId: 'anthropic/claude-haiku-4.5',
    })

    expect(createAgent).toHaveBeenCalledWith({
      name: 'research',
      model: 'openrouter/anthropic/claude-haiku-4.5',
    })
    expect(restart).not.toHaveBeenCalled()
  })

  it('merges custom openai-compatible providers before creating agents', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
      }),
      'utf-8',
    )
    await writeFile(join(tempDir, '.openclaw', '.env'), '', 'utf-8')

    const createAgent = mock(async () => ({
      agentId: 'research',
      name: 'research',
      workspace: `${OPENCLAW_CONTAINER_HOME}/workspace-research`,
      model: 'kimi-k2-5/accounts/fireworks/models/kimi-k2p5',
    }))
    const restart = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.restart = restart
    service.runtime = {
      isReady: async () => true,
    }
    service.cliClient = {
      createAgent,
    }

    await service.createAgent({
      name: 'research',
      providerType: 'openai-compatible',
      providerName: 'Kimi K2.5',
      baseUrl: 'https://api.fireworks.ai/inference/v1',
      apiKey: 'custom-key',
      modelId: 'accounts/fireworks/models/kimi-k2p5',
    })

    expect(restart).toHaveBeenCalledTimes(1)
    expect(createAgent).toHaveBeenCalledWith({
      name: 'research',
      model: 'kimi-k2-5/accounts/fireworks/models/kimi-k2p5',
    })
    expect(
      await readFile(join(tempDir, '.openclaw', '.env'), 'utf-8'),
    ).toContain('KIMI_K2_5_API_KEY=custom-key')
    expect(
      JSON.parse(
        await readFile(join(tempDir, '.openclaw', 'openclaw.json'), 'utf-8'),
      ),
    ).toMatchObject({
      gateway: {
        auth: {
          token: 'cli-token',
        },
      },
      models: {
        mode: 'merge',
        providers: {
          'kimi-k2-5': {
            api: 'openai-completions',
            baseUrl: 'https://api.fireworks.ai/inference/v1',
            apiKey: `\${KIMI_K2_5_API_KEY}`,
            models: [
              {
                id: 'accounts/fireworks/models/kimi-k2p5',
                name: 'accounts/fireworks/models/kimi-k2p5',
              },
            ],
          },
        },
      },
    })
  })

  it('preserves previously-registered custom provider models when re-registering an existing model', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
        models: {
          mode: 'merge',
          providers: {
            'kimi-k2-5': {
              api: 'openai-completions',
              baseUrl: 'https://api.fireworks.ai/inference/v1',
              apiKey: `\${KIMI_K2_5_API_KEY}`,
              models: [
                {
                  id: 'accounts/fireworks/models/kimi-k2p5',
                  name: 'accounts/fireworks/models/kimi-k2p5',
                },
                {
                  id: 'accounts/fireworks/models/kimi-k2p5-thinking',
                  name: 'accounts/fireworks/models/kimi-k2p5-thinking',
                },
              ],
            },
          },
        },
      }),
      'utf-8',
    )
    await writeFile(join(tempDir, '.openclaw', '.env'), '', 'utf-8')

    const createAgent = mock(async () => ({
      agentId: 'research',
      name: 'research',
      workspace: `${OPENCLAW_CONTAINER_HOME}/workspace-research`,
      model: 'kimi-k2-5/accounts/fireworks/models/kimi-k2p5',
    }))
    const restart = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.restart = restart
    service.runtime = {
      isReady: async () => true,
    }
    service.cliClient = {
      createAgent,
    }

    await service.createAgent({
      name: 'research',
      providerType: 'openai-compatible',
      providerName: 'Kimi K2.5',
      baseUrl: 'https://api.fireworks.ai/inference/v1',
      apiKey: 'custom-key',
      modelId: 'accounts/fireworks/models/kimi-k2p5',
    })

    expect(
      JSON.parse(
        await readFile(join(tempDir, '.openclaw', 'openclaw.json'), 'utf-8'),
      ),
    ).toMatchObject({
      models: {
        mode: 'merge',
        providers: {
          'kimi-k2-5': {
            models: [
              {
                id: 'accounts/fireworks/models/kimi-k2p5',
                name: 'accounts/fireworks/models/kimi-k2p5',
              },
              {
                id: 'accounts/fireworks/models/kimi-k2p5-thinking',
                name: 'accounts/fireworks/models/kimi-k2p5-thinking',
              },
            ],
          },
        },
      },
    })
  })

  it('updateProviderKeys rejects unsupported providers without restarting', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    const restart = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.restart = restart

    await expect(
      service.updateProviderKeys({
        providerType: 'google',
        apiKey: 'google-key',
        modelId: 'gemini-2.5-pro',
      }),
    ).rejects.toThrow('Unsupported OpenClaw provider')

    expect(restart).not.toHaveBeenCalled()
  })

  it('updateProviderKeys restores custom openai-compatible providers and restarts once', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', 'openclaw.json'),
      JSON.stringify({
        gateway: {
          auth: {
            token: 'cli-token',
          },
        },
      }),
      'utf-8',
    )
    await writeFile(join(tempDir, '.openclaw', '.env'), '', 'utf-8')

    const restart = mock(async () => {})
    const setDefaultModel = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.restart = restart
    service.runtime = {
      isReady: async () => true,
      waitForReady: mock(async () => true),
    }
    service.cliClient = {
      setDefaultModel,
    }

    const result = await service.updateProviderKeys({
      providerType: 'openai-compatible',
      providerName: 'Kimi K2.5',
      baseUrl: 'https://api.fireworks.ai/inference/v1',
      apiKey: 'custom-key',
      modelId: 'accounts/fireworks/models/kimi-k2p5',
    })

    expect(result).toEqual({
      restarted: true,
      modelUpdated: true,
    })
    expect(restart).toHaveBeenCalledTimes(1)
    expect(setDefaultModel).toHaveBeenCalledWith(
      'kimi-k2-5/accounts/fireworks/models/kimi-k2p5',
    )
    expect(
      await readFile(join(tempDir, '.openclaw', '.env'), 'utf-8'),
    ).toContain('KIMI_K2_5_API_KEY=custom-key')
    expect(
      JSON.parse(
        await readFile(join(tempDir, '.openclaw', 'openclaw.json'), 'utf-8'),
      ),
    ).toMatchObject({
      models: {
        mode: 'merge',
        providers: {
          'kimi-k2-5': {
            api: 'openai-completions',
            baseUrl: 'https://api.fireworks.ai/inference/v1',
            apiKey: `\${KIMI_K2_5_API_KEY}`,
          },
        },
      },
    })
  })

  it('does not restart when provider env content is unchanged', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', '.env'),
      'OPENAI_API_KEY=sk-test\n',
      'utf-8',
    )

    const restart = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.restart = restart

    await service.updateProviderKeys({
      providerType: 'openai',
      apiKey: 'sk-test',
    })

    expect(restart).not.toHaveBeenCalled()
    expect(await readFile(join(tempDir, '.openclaw', '.env'), 'utf-8')).toBe(
      'OPENAI_API_KEY=sk-test\n',
    )
  })

  it('applies the default model when provider keys are unchanged', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })
    await writeFile(
      join(tempDir, '.openclaw', '.env'),
      'OPENAI_API_KEY=sk-test\n',
      'utf-8',
    )

    const restart = mock(async () => {})
    const setDefaultModel = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.restart = restart
    service.runtime = {
      isReady: async () => true,
      waitForReady: async () => true,
    }
    service.cliClient = {
      setDefaultModel,
    }

    await expect(
      service.updateProviderKeys({
        providerType: 'openai',
        apiKey: 'sk-test',
        modelId: 'gpt-5.4-mini',
      }),
    ).resolves.toEqual({
      modelUpdated: true,
      restarted: false,
    })

    expect(setDefaultModel).toHaveBeenCalledWith('openai/gpt-5.4-mini')
    expect(restart).not.toHaveBeenCalled()
    expect(await readFile(join(tempDir, '.openclaw', '.env'), 'utf-8')).toBe(
      'OPENAI_API_KEY=sk-test\n',
    )
  })

  it('persists env updates before surfacing default-model failures', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'openclaw-service-'))
    await mkdir(join(tempDir, '.openclaw'), { recursive: true })

    const setDefaultModel = mock(async () => {
      throw new Error('container unavailable')
    })
    const restart = mock(async () => {})
    const service = new OpenClawService() as MutableOpenClawService

    service.openclawDir = tempDir
    service.restart = restart
    service.cliClient = {
      setDefaultModel,
    }

    await expect(
      service.updateProviderKeys({
        providerType: 'openai',
        apiKey: 'sk-test',
        modelId: 'gpt-5.4-mini',
      }),
    ).rejects.toThrow('container unavailable')

    expect(setDefaultModel).toHaveBeenCalledWith('openai/gpt-5.4-mini')
    expect(restart).toHaveBeenCalledTimes(1)
    expect(await readFile(join(tempDir, '.openclaw', '.env'), 'utf-8')).toBe(
      'OPENAI_API_KEY=sk-test\n',
    )
  })
})

function mockGatewayAuth(status = 200): ReturnType<typeof mock> {
  const fetchMock = mock(() => Promise.resolve(new Response('', { status })))
  globalThis.fetch = fetchMock as typeof globalThis.fetch
  return fetchMock
}
