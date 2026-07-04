/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * BrowserOS Server Application
 *
 * Manages server lifecycle: initialization, startup, and shutdown.
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
  configureClaudeRuntime,
  configureCodexRuntime,
} from './lib/agents/runtime'
import {
  cleanOldSessions,
  ensureBrowserosDir,
  getDbPath,
  removeServerConfigSync,
  writeServerConfig,
} from './lib/browseros-dir'
import { initializeDb } from './lib/db'
import { identity } from './lib/identity'
import { logger } from './lib/logger'
import { reconcileUrl } from './lib/mcp-manager'
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

    configureClaudeRuntime()
    configureCodexRuntime()
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

    // Reconcile every linked agent's BrowserOS MCP URL against the
    // proxy URL external clients actually reach. The agent server's
    // own `serverPort` is NOT that URL — in production the browser
    // proxies `/mcp` from a separately-configured proxy port. We
    // only reconcile when the launching process passes the public
    // URL via `BROWSEROS_MCP_PUBLIC_URL`; otherwise we'd rewrite
    // every agent config with the wrong port and break installs that
    // were previously working. The UI's install flow records the
    // correct URL per click; reconcile is the boot-time recovery
    // path for port drift.
    const publicMcpUrl = process.env.BROWSEROS_MCP_PUBLIC_URL
    if (publicMcpUrl) {
      reconcileUrl({ currentUrl: publicMcpUrl }).catch((err) => {
        logger.warn(
          'MCP manager URL reconcile failed; agent configs may be stale',
          {
            error: err instanceof Error ? err.message : String(err),
          },
        )
      })
    } else {
      logger.debug(
        'Skipping MCP manager URL reconcile — BROWSEROS_MCP_PUBLIC_URL not set',
      )
    }

    logger.info(
      `HTTP server listening on http://127.0.0.1:${this.config.serverPort}`,
    )
    logger.info(
      `Health endpoint: http://127.0.0.1:${this.config.serverPort}/health`,
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

    identity.initialize({
      installId: this.config.instanceInstallId,
      statePath: path.join(
        this.config.executionDir,
        'identity',
        'browseros-id.json',
      ),
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
    } else if (
      !this.config.instanceClientId &&
      !this.config.instanceInstallId
    ) {
      // captureNow short-circuits when no identity is set, so emits
      // will silently no-op until the deployment supplies one of these.
      // Surface the cause so a misconfigured instance doesn't quietly
      // produce zero analytics.
      logger.warn(
        'Metrics will skip events: no instance identity. ' +
          'Set instance.client_id or instance.install_id in the sidecar config to opt in.',
      )
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
