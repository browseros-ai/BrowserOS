/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it, mock, spyOn } from 'bun:test'

const config = {
  cdpPort: 9222,
  serverPort: 9100,
  agentPort: 9100,
  extensionPort: null,
  resourcesDir: '/tmp/browseros-resources',
  executionDir: '/tmp/browseros-execution',
  mcpAllowRemote: false,
  aiSdkDevtoolsEnabled: false,
  instanceClientId: 'client-test',
}

describe('Application.start', () => {
  afterEach(() => {
    mock.restore()
    mock.clearAllMocks()
  })

  it('starts with the CDP backend only', async () => {
    const {
      Application,
      browserModule,
      cdpConnect,
      createHttpServer,
      loggerError,
      loggerInfo,
      loggerWarn,
    } = await setupApplicationTest()
    const app = new Application(config)

    await app.start()

    expect(cdpConnect).toHaveBeenCalledTimes(1)
    expect(createHttpServer).toHaveBeenCalledTimes(1)
    expect(createHttpServer.mock.calls[0]?.[0]).toEqual(
      expect.objectContaining({
        browser: expect.any(browserModule.Browser),
      }),
    )
    expect(createHttpServer.mock.calls[0]?.[0]).not.toHaveProperty('controller')
    expect(loggerInfo).toHaveBeenCalled()
    expect(loggerWarn).not.toHaveBeenCalled()
    expect(loggerError).not.toHaveBeenCalled()
  })

  it('does not start the Hermes runtime on startup', async () => {
    const {
      Application,
      configureHermesRuntime,
      createHttpServer,
      hermesService,
    } = await setupApplicationTest()
    const app = new Application(config)

    await app.start()

    expect(createHttpServer).toHaveBeenCalledTimes(1)
    expect(configureHermesRuntime).not.toHaveBeenCalled()
    expect(hermesService.executeAction).not.toHaveBeenCalled()
  })

  it('stores the database below the BrowserOS directory instead of the execution directory', async () => {
    const originalBrowserosDir = process.env.BROWSEROS_DIR
    process.env.BROWSEROS_DIR = '/tmp/browseros-dogfood'

    try {
      const { Application, initializeDb } = await setupApplicationTest()
      const app = new Application(config)

      await app.start()

      expect(initializeDb).toHaveBeenCalledWith({
        dbPath: '/tmp/browseros-dogfood/db/browseros.sqlite',
        resourcesDir: config.resourcesDir,
      })
    } finally {
      if (originalBrowserosDir === undefined) {
        delete process.env.BROWSEROS_DIR
      } else {
        process.env.BROWSEROS_DIR = originalBrowserosDir
      }
    }
  })

  it('warns at boot when metrics is enabled but no instance identity is configured', async () => {
    const { Application, loggerWarn } = await setupApplicationTest()
    const { instanceClientId: _unused, ...configWithoutIdentity } = config
    const app = new Application(configWithoutIdentity)

    await app.start()

    const warnedAboutIdentity = loggerWarn.mock.calls.some(
      (args) =>
        typeof args[0] === 'string' && args[0].includes('no instance identity'),
    )
    expect(warnedAboutIdentity).toBe(true)
  })
})

async function setupApplicationTest() {
  const apiServer = await import('../src/api/server')
  const browserModule = await import('../src/browser/browser')
  const cdpModule = await import('../src/browser/backends/cdp')
  const runtimeModule = await import('../src/lib/agents/runtime')
  const browserosDir = await import('../src/lib/browseros-dir')
  const dbModule = await import('../src/lib/db')
  const identityModule = await import('../src/lib/identity')
  const loggerModule = await import('../src/lib/logger')
  const metricsModule = await import('../src/lib/metrics')
  const sentryModule = await import('../src/lib/sentry')

  const createHttpServer = spyOn(apiServer, 'createHttpServer')
  createHttpServer.mockImplementation(async () => ({}) as never)

  const cdpConnect = mock(async () => {})
  spyOn(cdpModule.CdpBackend.prototype, 'connect').mockImplementation(
    cdpConnect,
  )

  spyOn(browserosDir, 'cleanOldSessions').mockImplementation(async () => {})
  spyOn(browserosDir, 'ensureBrowserosDir').mockImplementation(async () => {})
  spyOn(browserosDir, 'writeServerConfig').mockImplementation(async () => {})
  spyOn(browserosDir, 'removeServerConfigSync').mockImplementation(() => {})

  const initializeDb = spyOn(dbModule, 'initializeDb').mockImplementation(
    () =>
      ({
        path: '/tmp/browseros-state/db/browseros.sqlite',
        migrationsDir: '/tmp/browseros-resources/db/migrations',
        sqlite: { close: () => {} },
        db: {},
      }) as never,
  )
  spyOn(identityModule.identity, 'initialize').mockImplementation(() => {})
  spyOn(identityModule.identity, 'getBrowserOSId').mockImplementation(
    () => 'browseros-id',
  )

  const loggerInfo = spyOn(loggerModule.logger, 'info').mockImplementation(
    () => {},
  )
  const loggerWarn = spyOn(loggerModule.logger, 'warn').mockImplementation(
    () => {},
  )
  spyOn(loggerModule.logger, 'debug').mockImplementation(() => {})
  const loggerError = spyOn(loggerModule.logger, 'error').mockImplementation(
    () => {},
  )
  spyOn(loggerModule.logger, 'setLogFile').mockImplementation(() => {})

  spyOn(metricsModule.metrics, 'initialize').mockImplementation(() => {})
  spyOn(metricsModule.metrics, 'isEnabled').mockImplementation(() => true)
  spyOn(metricsModule.metrics, 'log').mockImplementation(() => {})

  spyOn(sentryModule.Sentry, 'setContext').mockImplementation(() => {})
  spyOn(sentryModule.Sentry, 'setUser').mockImplementation(() => {})
  spyOn(sentryModule.Sentry, 'captureException').mockImplementation(() => {})

  const hermesExecuteAction = mock(async () => {})
  const fakeHermesRuntime = { executeAction: hermesExecuteAction } as never
  const configureHermesRuntime = spyOn(
    runtimeModule,
    'configureHermesRuntime',
  ).mockImplementation(() => fakeHermesRuntime)
  spyOn(runtimeModule, 'getHermesRuntime').mockImplementation(
    () => fakeHermesRuntime,
  )
  spyOn(runtimeModule, 'configureClaudeRuntime').mockImplementation(
    () => ({}) as never,
  )
  spyOn(runtimeModule, 'configureCodexRuntime').mockImplementation(
    () => ({}) as never,
  )

  const { Application } = await import('../src/main')
  return {
    Application,
    browserModule,
    cdpConnect,
    createHttpServer,
    loggerError,
    loggerInfo,
    loggerWarn,
    initializeDb,
    configureHermesRuntime,
    hermesService: { executeAction: hermesExecuteAction },
  }
}
