/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it, mock } from 'bun:test'

const config = {
  cdpPort: 9222,
  serverPort: 9100,
  agentPort: 9100,
  extensionPort: null,
  resourcesDir: '/tmp/browseros-resources',
  executionDir: '/tmp/browseros-execution',
  mcpAllowRemote: false,
  aiSdkDevtoolsEnabled: false,
}

describe('Application.start', () => {
  afterEach(() => {
    mock.restore()
  })

  it('starts with the CDP backend only', async () => {
    const createHttpServer = mock(async () => ({}))
    const cdpConnect = mock(async () => {})
    const browserCtor = mock(() => {})
    const loggerInfo = mock(() => {})
    const loggerWarn = mock(() => {})
    const loggerDebug = mock(() => {})
    const loggerError = mock(() => {})
    mock.module('../src/api/server', () => ({
      createHttpServer,
    }))
    mock.module('../src/browser/backends/cdp', () => ({
      CdpBackend: class {
        async connect(): Promise<void> {
          await cdpConnect()
        }
      },
    }))
    mock.module('../src/browser/browser', () => ({
      Browser: class {
        constructor(cdp: unknown) {
          browserCtor(cdp)
        }
      },
    }))
    mock.module('../src/lib/browseros-dir', () => ({
      cleanOldSessions: mock(async () => {}),
      ensureBrowserosDir: mock(async () => {}),
      removeServerConfigSync: mock(() => {}),
      writeServerConfig: mock(async () => {}),
    }))
    mock.module('../src/lib/db', () => ({
      initializeDb: mock(() => ({})),
    }))
    mock.module('../src/lib/identity', () => ({
      identity: {
        initialize: mock(() => {}),
        getBrowserOSId: mock(() => 'browseros-id'),
      },
    }))
    mock.module('../src/lib/logger', () => ({
      logger: {
        setLogFile: mock(() => {}),
        info: loggerInfo,
        warn: loggerWarn,
        debug: loggerDebug,
        error: loggerError,
      },
    }))
    mock.module('../src/lib/metrics', () => ({
      metrics: {
        initialize: mock(() => {}),
        isEnabled: mock(() => true),
        log: mock(() => {}),
      },
    }))
    mock.module('../src/lib/sentry', () => ({
      Sentry: {
        setContext: mock(() => {}),
        setUser: mock(() => {}),
        captureException: mock(() => {}),
      },
    }))
    mock.module('../src/lib/soul', () => ({
      seedSoulTemplate: mock(async () => {}),
    }))
    mock.module('../src/skills/migrate', () => ({
      migrateBuiltinSkills: mock(async () => {}),
    }))
    mock.module('../src/skills/remote-sync', () => ({
      startSkillSync: mock(() => {}),
      stopSkillSync: mock(() => {}),
      syncBuiltinSkills: mock(async () => {}),
    }))
    mock.module('../src/tools/registry', () => ({
      registry: {
        names: () => ['test_tool'],
      },
    }))

    const { Application } = await import('../src/main')
    const app = new Application(config)

    await app.start()

    expect(cdpConnect).toHaveBeenCalledTimes(1)
    expect(browserCtor).toHaveBeenCalledTimes(1)
    expect(createHttpServer).toHaveBeenCalledTimes(1)
    expect(createHttpServer.mock.calls[0]?.[0]).not.toHaveProperty('controller')
    expect(loggerWarn).not.toHaveBeenCalled()
    expect(loggerError).not.toHaveBeenCalled()
  })
})
