/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * BrowserOS Server Application
 *
 * Manages server lifecycle: initialization, startup, and shutdown.
 */

import type { Database } from 'bun:sqlite'
import fs from 'node:fs'
import path from 'node:path'
import { EXIT_CODES } from '@browseros/shared/constants/exit-codes'
import { createHttpServer } from './api/server'
import {
  configureOpenClawService,
  getOpenClawService,
} from './api/services/openclaw/openclaw-service'
import { configurePodmanRuntime } from './api/services/openclaw/podman-runtime'
import { CdpBackend } from './browser/backends/cdp'
import { Browser } from './browser/browser'
import type { ServerConfig } from './config'
import { INLINED_ENV } from './env'
import {
  cleanOldSessions,
  ensureBrowserosDir,
  removeServerConfigSync,
  writeServerConfig,
} from './lib/browseros-dir'
import { initializeDb } from './lib/db'
import { identity } from './lib/identity'
import { logger } from './lib/logger'
import { metrics } from './lib/metrics'
import { isPortInUseError } from './lib/port-binding'
import { Sentry } from './lib/sentry'
import { seedSoulTemplate } from './lib/soul'
import { migrateBuiltinSkills } from './skills/migrate'
import {
  startSkillSync,
  stopSkillSync,
  syncBuiltinSkills,
} from './skills/remote-sync'
import { registry } from './tools/registry'
import { VERSION } from './version'

export class Application {
  private config: ServerConfig
  private db: Database | null = null

  constructor(config: ServerConfig) {
    this.config = config
  }

  async start(): Promise<void> {
    logger.info(`Starting BrowserOS Server v${VERSION}`)
    logger.debug('Directory config', {
      executionDir: path.resolve(this.config.executionDir),
      resourcesDir: path.resolve(this.config.resourcesDir),
    })

    configurePodmanRuntime({
      resourcesDir: path.resolve(this.config.resourcesDir),
    })
    await this.initCoreServices()

    if (!this.config.cdpPort) {
      logger.error('CDP port is required (--cdp-port)')
      process.exit(EXIT_CODES.GENERAL_ERROR)
    }

    const cdp = new CdpBackend({ port: this.config.cdpPort })
    try {
      logger.debug(`Connecting to CDP on port ${this.config.cdpPort}`)
      await cdp.connect()
      logger.info(`Connected to CDP on port ${this.config.cdpPort}`)
    } catch (error) {
      return this.handleStartupError('CDP', this.config.cdpPort, error)
    }

    const browser = new Browser(cdp)

    logger.info(`Loaded ${registry.names().length} unified tools`)

    try {
      await createHttpServer({
        port: this.config.serverPort,
        host: '0.0.0.0',
        version: VERSION,
        browser,
        registry,
        browserosId: identity.getBrowserOSId(),
        executionDir: this.config.executionDir,
        resourcesDir: this.config.resourcesDir,
        codegenServiceUrl: this.config.codegenServiceUrl,
        aiSdkDevtoolsEnabled: this.config.aiSdkDevtoolsEnabled,

        onShutdown: () => this.stop('shutdown-endpoint'),
      })
    } catch (error) {
      this.handleStartupError('HTTP server', this.config.serverPort, error)
    }

    try {
      await writeServerConfig({
        server_port: this.config.serverPort,
        url: `http://127.0.0.1:${this.config.serverPort}`,
        server_version: VERSION,
        browseros_version: this.config.instanceBrowserosVersion,
        chromium_version: this.config.instanceChromiumVersion,
        browseros_id: identity.getBrowserOSId(),
      })
    } catch (error) {
      logger.warn('Failed to write server config for auto-discovery', {
        error: error instanceof Error ? error.message : String(error),
      })
    }

    logger.info(
      `HTTP server listening on http://127.0.0.1:${this.config.serverPort}`,
    )
    logger.info(
      `Health endpoint: http://127.0.0.1:${this.config.serverPort}/health`,
    )

    this.logStartupSummary()
    startSkillSync()

    configureOpenClawService({
      browserosServerPort: this.config.serverPort,
      resourcesDir: path.resolve(this.config.resourcesDir),
    })
      .tryAutoStart()
      .catch((err) =>
        logger.warn('OpenClaw auto-start failed', {
          error: err instanceof Error ? err.message : String(err),
        }),
      )

    metrics.log('http_server.started', { version: VERSION })
  }

  stop(reason?: string): void {
    logger.info('Shutting down server...', { reason })
    stopSkillSync()
    getOpenClawService()
      .shutdown()
      .catch(() => {})
    removeServerConfigSync()

    // Immediate exit without graceful shutdown. Chromium may kill us on update/restart,
    // and we need to free the port instantly so the HTTP port doesn't keep switching.
    // Exit 0 only for managed shutdowns (POST /shutdown from Chromium).
    // Signal kills exit non-zero so Chromium's OnProcessExited restarts us.
    const code =
      reason === 'SIGTERM' || reason === 'SIGINT'
        ? EXIT_CODES.SIGNAL_KILL
        : EXIT_CODES.SUCCESS
    process.exit(code)
  }

  private async initCoreServices(): Promise<void> {
    this.configureLogDirectory()
    await ensureBrowserosDir()
    await cleanOldSessions()
    await seedSoulTemplate()
    await migrateBuiltinSkills()
    await syncBuiltinSkills()

    const dbPath = path.join(
      this.config.executionDir || this.config.resourcesDir,
      'browseros.db',
    )
    this.db = initializeDb(dbPath)

    identity.initialize({
      installId: this.config.instanceInstallId,
      db: this.db,
    })

    const browserosId = identity.getBrowserOSId()
    logger.info('BrowserOS ID initialized', {
      browserosId: browserosId.slice(0, 12),
      fromConfig: !!this.config.instanceInstallId,
    })

    metrics.initialize({
      client_id: this.config.instanceClientId,
      install_id: this.config.instanceInstallId,
      browseros_version: this.config.instanceBrowserosVersion,
      chromium_version: this.config.instanceChromiumVersion,
      server_version: VERSION,
    })

    if (!metrics.isEnabled()) {
      logger.warn('Metrics disabled: missing POSTHOG_API_KEY')
    }

    if (!INLINED_ENV.SENTRY_DSN) {
      logger.debug('Sentry disabled: missing SENTRY_DSN')
    }

    Sentry.setUser({ id: browserosId })
    Sentry.setContext('browseros', {
      client_id: this.config.instanceClientId,
      install_id: this.config.instanceInstallId,
      browseros_version: this.config.instanceBrowserosVersion,
      chromium_version: this.config.instanceChromiumVersion,
      server_version: VERSION,
    })
  }

  private configureLogDirectory(): void {
    const logDir = this.config.executionDir
    const resolvedDir = path.isAbsolute(logDir)
      ? logDir
      : path.resolve(process.cwd(), logDir)

    try {
      fs.mkdirSync(resolvedDir, { recursive: true })
      logger.setLogFile(resolvedDir)
    } catch (error) {
      console.warn(
        `Failed to configure log directory ${resolvedDir}: ${
          error instanceof Error ? error.message : String(error)
        }`,
      )
    }
  }

  private handleStartupError(
    serverName: string,
    port: number,
    error: unknown,
  ): never {
    const errorMsg = error instanceof Error ? error.message : String(error)
    logger.error(`Failed to start ${serverName}`, { port, error: errorMsg })
    console.error(
      `[FATAL] Failed to start ${serverName} on port ${port}: ${errorMsg}`,
    )

    if (isPortInUseError(error)) {
      console.error(
        `[FATAL] Port ${port} is already in use. Chromium should try a different port.`,
      )
      process.exit(EXIT_CODES.PORT_CONFLICT)
    }

    Sentry.captureException(error)
    process.exit(EXIT_CODES.GENERAL_ERROR)
  }

  private logStartupSummary(): void {
    logger.info('')
    logger.info('Services running:')
    logger.info(`  HTTP Server: http://127.0.0.1:${this.config.serverPort}`)
    logger.info('')
  }
}
