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
  vmCachePrefetch: true,
  vmCacheManifestUrl: 'https://cdn.browseros.com/vm/manifest.json',
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

  it('starts VM cache prefetch without blocking HTTP startup', async () => {
    const { Application, createHttpServer, prefetchVmCache } =
      await setupApplicationTest()
    let resolvePrefetch: (value: {
      downloaded: string[]
      manifestPath: string
      skipped: boolean
    }) => void = () => {}
    const pendingPrefetch = new Promise<{
      downloaded: string[]
      manifestPath: string
      skipped: boolean
    }>((resolve) => {
      resolvePrefetch = resolve
    })
    prefetchVmCache.mockImplementation(() => pendingPrefetch)

    const app = new Application(config)
    const startPromise = app.start()
    const completedBeforePrefetch = await Promise.race([
      startPromise.then(() => true),
      Bun.sleep(25).then(() => false),
    ])
    resolvePrefetch({
      downloaded: [],
      manifestPath: '/tmp/manifest.json',
      skipped: true,
    })
    await startPromise

    expect(completedBeforePrefetch).toBe(true)
    expect(createHttpServer).toHaveBeenCalledTimes(1)
    expect(prefetchVmCache).toHaveBeenCalledWith({
      manifestUrl: 'https://cdn.browseros.com/vm/manifest.json',
    })
  })

  it('logs VM cache prefetch failures without failing startup', async () => {
    const { Application, createHttpServer, loggerWarn, prefetchVmCache } =
      await setupApplicationTest()
    prefetchVmCache.mockImplementation(() =>
      Promise.reject(new Error('cache offline')),
    )
    const app = new Application(config)

    await app.start()
    await Bun.sleep(0)

    expect(createHttpServer).toHaveBeenCalledTimes(1)
    expect(loggerWarn).toHaveBeenCalledWith(
      'BrowserOS VM cache prefetch failed',
      {
        error: 'cache offline',
      },
    )
  })

  it('skips VM cache prefetch when disabled', async () => {
    const { Application, prefetchVmCache } = await setupApplicationTest()
    const app = new Application({ ...config, vmCachePrefetch: false })

    await app.start()

    expect(prefetchVmCache).not.toHaveBeenCalled()
  })
})

async function setupApplicationTest() {
  const apiServer = await import('../src/api/server')
  const browserModule = await import('../src/browser/browser')
  const cdpModule = await import('../src/browser/backends/cdp')
  const openclawService = await import(
    '../src/api/services/openclaw/openclaw-service'
  )
  const browserosDir = await import('../src/lib/browseros-dir')
  const cacheSync = await import('../src/lib/vm/cache-sync')
  const dbModule = await import('../src/lib/db')
  const identityModule = await import('../src/lib/identity')
  const loggerModule = await import('../src/lib/logger')
  const metricsModule = await import('../src/lib/metrics')
  const sentryModule = await import('../src/lib/sentry')
  const soulModule = await import('../src/lib/soul')
  const migrateModule = await import('../src/skills/migrate')
  const remoteSyncModule = await import('../src/skills/remote-sync')

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

  spyOn(dbModule, 'initializeDb').mockImplementation(() => ({}) as never)
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

  spyOn(soulModule, 'seedSoulTemplate').mockImplementation(async () => {})
  spyOn(migrateModule, 'migrateBuiltinSkills').mockImplementation(
    async () => {},
  )
  spyOn(remoteSyncModule, 'syncBuiltinSkills').mockImplementation(
    async () => {},
  )
  spyOn(remoteSyncModule, 'startSkillSync').mockImplementation(() => {})
  spyOn(remoteSyncModule, 'stopSkillSync').mockImplementation(() => {})

  spyOn(openclawService, 'configureVmRuntime').mockImplementation(
    () =>
      ({
        tryAutoStart: async () => {},
      }) as never,
  )
  spyOn(openclawService, 'configureOpenClawService').mockImplementation(
    () =>
      ({
        tryAutoStart: async () => {},
      }) as never,
  )

  const prefetchVmCache = spyOn(cacheSync, 'prefetchVmCache')
  prefetchVmCache.mockImplementation(async () => ({
    downloaded: [],
    manifestPath: '/tmp/manifest.json',
    skipped: true,
  }))

  const { Application } = await import('../src/main')
  return {
    Application,
    browserModule,
    cdpConnect,
    createHttpServer,
    loggerError,
    loggerInfo,
    loggerWarn,
    prefetchVmCache,
  }
}
