/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import fs from 'node:fs'
import path from 'node:path'
import { CdpBackend } from '@browseros/browser-core/backends/cdp'
import { Browser } from '@browseros/browser-core/browser'
import { EXIT_CODES } from '@browseros/shared/constants/exit-codes'
import { createHttpServer } from './api/server'
import type { ServerConfig } from './config'
import { INLINED_ENV } from './env'
import {
  cleanOldSessions,
  ensureBrowserosDir,
  getBrowserosDir,
  getDbPath,
  removeServerConfigSync,
  writeServerConfig,
} from './lib/browseros-dir'
import { initializeDb } from './lib/db'
import { identity } from './lib/identity'
import { loadOrCreateInstallationId } from './lib/installation-id'
import { logger } from './lib/logger'
import { selfHealMcpLinks } from './lib/mcp-manager'
import { metrics } from './lib/metrics'
import { isPortInUseError } from './lib/port-binding'
import { Sentry } from './lib/sentry'
import { VERSION } from './version'

export class Application {
  private config: ServerConfig

  constructor(config: ServerConfig) {
    this.config = config
  }

  async start(): Promise<void> {
    logger.info(`Starting BrowserOS Server v${VERSION}`)
    logger.debug('Directory config', {
      executionDir: path.resolve(this.config.executionDir),
      resourcesDir: path.resolve(this.config.resourcesDir),
    })

    await this.initCoreServices()

    if (!this.config.cdpPort) {
      logger.error('CDP port is required in the sidecar config')
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
    const browserSession = browser.session

    try {
      await createHttpServer({
        port: this.config.serverPort,
        host: '0.0.0.0',
        version: VERSION,
        browser,
        browserSession,
        browserosId: identity.getBrowserOSId(),
        executionDir: this.config.executionDir,
        resourcesDir: this.config.resourcesDir,
        aiSdkDevtoolsEnabled: this.config.aiSdkDevtoolsEnabled,
        onShutdown: () => this.stop('shutdown-endpoint'),
      })
    } catch (error) {
      this.handleStartupError('HTTP server', this.config.serverPort, error)
    }

    try {
      await writeServerConfig({
        server_port: this.config.serverPort,
        cdp_port: this.config.cdpPort ?? undefined,
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

    // Boot self-heal for the MCP integration. First drops BrowserOS
    // from any agent no longer in the curated surface, then repairs
    // every remaining agent's BrowserOS MCP URL against the proxy URL
    // external clients actually reach. The agent server's own
    // `serverPort` is NOT that URL: in production the browser proxies
    // `/mcp` from a separately-configured proxy port. The URL repair
    // only runs when the launching process passes the public URL via
    // `BROWSEROS_MCP_PUBLIC_URL`; otherwise we'd rewrite every agent
    // config with the wrong port. The non-curated cleanup needs no URL
    // and always runs. The UI's install flow records the correct URL
    // per click; this is the boot-time recovery path.
    const publicMcpUrl = process.env.BROWSEROS_MCP_PUBLIC_URL
    selfHealMcpLinks({ currentUrl: publicMcpUrl }).catch((err) => {
      logger.warn('MCP manager self-heal failed; agent configs may be stale', {
        error: err instanceof Error ? err.message : String(err),
      })
    })
    if (!publicMcpUrl) {
      logger.debug(
        'MCP manager URL reconcile skipped — BROWSEROS_MCP_PUBLIC_URL not set',
      )
    }

    logger.info(
      `HTTP server listening on http://127.0.0.1:${this.config.serverPort}`,
    )
    logger.info(
      `Health endpoint: http://127.0.0.1:${this.config.serverPort}/system/health`,
    )

    this.logStartupSummary()

    metrics.log('http_server.started', { version: VERSION })
  }

  stop(reason?: string): void {
    logger.info('Shutting down server...', { reason })
    removeServerConfigSync()

    // Immediate exit keeps the port free; signal exits stay non-zero so Chromium restarts us.
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

    initializeDb({
      dbPath: getDbPath(),
      resourcesDir: this.config.resourcesDir,
    })

    let installationId: string | undefined
    try {
      installationId = await loadOrCreateInstallationId(getBrowserosDir())
    } catch (error) {
      // Preserve malformed state instead of silently rotating identity. The
      // server remains usable with an ephemeral functional ID, while metrics
      // and Sentry correlation stay disabled until the file is repaired.
      logger.error('Installation identity unavailable', {
        error: error instanceof Error ? error.message : String(error),
      })
    }

    identity.initialize({
      installId: installationId,
    })

    const browserosId = identity.getBrowserOSId()
    logger.info('BrowserOS ID initialized', {
      browserosId: browserosId.slice(0, 12),
      durable: !!installationId,
    })

    metrics.initialize({
      install_id: installationId,
      browseros_version: this.config.instanceBrowserosVersion,
      chromium_version: this.config.instanceChromiumVersion,
      server_version: VERSION,
    })

    if (!metrics.isEnabled()) {
      logger.warn('Metrics disabled: missing POSTHOG_API_KEY')
    } else if (!installationId) {
      logger.warn(
        'Metrics will skip events: installation identity unavailable.',
      )
    }

    if (!INLINED_ENV.SENTRY_DSN) {
      logger.debug('Sentry disabled: missing SENTRY_DSN')
    }

    if (installationId) {
      Sentry.setUser({ id: installationId })
    }
    Sentry.setContext('browseros', {
      install_id: installationId,
      product: 'browseros',
      surface: 'server',
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
