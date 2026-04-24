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
}

describe('Application.start', () => {
  afterEach(() => {
    mock.restore()
    mock.clearAllMocks()
  })

  it('starts with the CDP backend only', async () => {
    const apiServer = await import('../src/api/server')
    const browserModule = await import('../src/browser/browser')
    const cdpModule = await import('../src/browser/backends/cdp')
    const browserosDir = await import('../src/lib/browseros-dir')
    const dbModule = await import('../src/lib/db')
    const identityModule = await import('../src/lib/identity')
    const loggerModule = await import('../src/lib/logger')
    const metricsModule = await import('../src/lib/metrics')
    const sentryModule = await import('../src/lib/sentry')
    const soulModule = await import('../src/lib/soul')
    const openclawService = await import(
      '../src/api/services/openclaw/openclaw-service'
    )
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

    const { Application } = await import('../src/main')
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
})
