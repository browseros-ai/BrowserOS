/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { KLAVIS_PROXY_RETRY_BACKOFF_MS } from '@browseros/shared/constants/timeouts'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { ToolSet } from 'ai'
import { logger } from '../../../lib/logger'
import {
  getConnectorCatalog,
  getConnectorServerNames,
  isSupportedConnector,
} from './catalog'
import { KlavisClient } from './client'
import {
  buildConnectorInventory,
  getAuthUrlForServer,
  selectedServerNames,
} from './connector-state'
import { KlavisStrataCache } from './strata-cache'
import {
  type ConnectKlavisStrataSessionDeps,
  connectKlavisStrataSession,
} from './strata-session'
import { buildKlavisToolSet, registerKlavisTools } from './tool-adapters'
import type {
  ConnectorConnectionIntent,
  ConnectorInventory,
  ConnectorToolScope,
  KlavisProxyStatus,
  KlavisSessionHandle,
  SubmitApiKeyInput,
  UserIntegration,
} from './types'

export interface KlavisServiceDeps {
  browserosId?: string | null
  client?: KlavisClient
  cache?: KlavisStrataCache
  connect?: (
    deps: ConnectKlavisStrataSessionDeps,
  ) => Promise<KlavisSessionHandle>
  retryDelaysMs?: readonly number[]
}

/** Owns Klavis catalog, auth, Strata lifecycle, cache, and tool exposure. */
export class KlavisService {
  private readonly browserosId?: string
  private readonly client: KlavisClient
  private readonly cache: KlavisStrataCache
  private readonly connect: (
    deps: ConnectKlavisStrataSessionDeps,
  ) => Promise<KlavisSessionHandle>
  private readonly retryDelaysMs: readonly number[]
  private retryTimer: ReturnType<typeof setTimeout> | undefined
  private session: KlavisSessionHandle | null = null
  private stopped = false
  private status: KlavisProxyStatus

  constructor(deps: KlavisServiceDeps) {
    this.browserosId = deps.browserosId ?? undefined
    this.client = deps.client ?? new KlavisClient()
    this.cache = deps.cache ?? new KlavisStrataCache()
    this.connect = deps.connect ?? connectKlavisStrataSession
    this.retryDelaysMs = deps.retryDelaysMs ?? KLAVIS_PROXY_RETRY_BACKOFF_MS
    this.status = this.browserosId
      ? { state: 'stopped' }
      : { state: 'disabled', reason: 'missing_browseros_id' }
  }

  /** Starts the background Strata session without blocking route startup. */
  start(): void {
    if (!this.browserosId || this.status.state === 'disabled') {
      return
    }
    if (
      this.status.state === 'connecting' ||
      this.status.state === 'retrying' ||
      this.status.state === 'ready'
    ) {
      return
    }

    this.stopped = false
    this.status = { state: 'connecting' }
    void this.attemptConnect(0)
  }

  /** Stops retry timers and closes the active Strata MCP session. */
  async stop(): Promise<void> {
    this.stopped = true
    if (this.retryTimer) {
      clearTimeout(this.retryTimer)
      this.retryTimer = undefined
    }
    const session = this.session
    this.session = null
    this.status = this.browserosId
      ? { state: 'stopped' }
      : { state: 'disabled', reason: 'missing_browseros_id' }
    await session?.close().catch((error) => {
      logger.warn('Failed to close Klavis proxy transport', {
        error: error instanceof Error ? error.message : String(error),
      })
    })
  }

  getProxyStatus(): KlavisProxyStatus {
    return this.status
  }

  listAvailableConnectors() {
    return getConnectorCatalog()
  }

  async getUserIntegrations(): Promise<UserIntegration[]> {
    const browserosId = this.requireBrowserosId()
    return this.client.getUserIntegrations(browserosId)
  }

  async getConnectorInventory(
    scope?: ConnectorToolScope,
  ): Promise<ConnectorInventory> {
    const integrations = this.browserosId
      ? await this.getUserIntegrations()
      : []
    return buildConnectorInventory({
      available: this.listAvailableConnectors(),
      integrations,
      proxy: this.getProxyStatus(),
      scope,
    })
  }

  async createConnectionIntent(
    serverName: string,
  ): Promise<ConnectorConnectionIntent> {
    const [intent] = await this.createConnectionIntents([serverName])
    if (!intent) {
      throw new Error(`Invalid server: ${serverName}`)
    }
    const browserosId = this.requireBrowserosId()
    this.cache.invalidate(browserosId)
    return intent
  }

  async createConnectionIntents(
    serverNames: readonly string[],
  ): Promise<ConnectorConnectionIntent[]> {
    const browserosId = this.requireBrowserosId()
    for (const serverName of serverNames) {
      this.requireSupportedConnector(serverName)
    }
    const result = await this.client.createStrata(browserosId, [...serverNames])
    return serverNames.map((serverName) => ({
      serverName,
      strataId: result.strataId,
      addedServers: result.addedServers,
      oauthUrl: getAuthUrlForServer(result.oauthUrls, serverName),
      apiKeyUrl: getAuthUrlForServer(result.apiKeyUrls, serverName),
    }))
  }

  async submitApiKey(input: SubmitApiKeyInput): Promise<void> {
    const browserosId = this.requireBrowserosId()
    this.requireSupportedConnector(input.serverName)
    await this.client.submitApiKey(input.apiKeyUrl, input.apiKey)
    this.cache.invalidate(browserosId)
  }

  async removeConnector(serverName: string): Promise<void> {
    const browserosId = this.requireBrowserosId()
    this.requireSupportedConnector(serverName)
    const strata = await this.client.createStrata(browserosId, [serverName])
    await this.client.deleteServersFromStrata(strata.strataId, [serverName])
    this.cache.invalidate(browserosId)
  }

  buildAiSdkToolSet(scope: ConnectorToolScope = {}): ToolSet {
    if (!this.browserosId) {
      return {}
    }
    return buildKlavisToolSet(this.toolAdapterDeps(scope))
  }

  registerMcpTools(server: McpServer, scope: ConnectorToolScope = {}): void {
    if (!this.browserosId) {
      logger.debug('Skipping Klavis MCP tools registration', {
        reason: 'missing_browseros_id',
        selectedServers: selectedServerNames(scope),
      })
      return
    }
    logger.debug('Registering Klavis MCP tools', {
      proxyState: this.status.state,
      selectedServers: selectedServerNames(scope),
      sessionToolCount: this.session?.tools.length ?? 0,
    })
    registerKlavisTools(server, this.toolAdapterDeps(scope))
  }

  private async attemptConnect(attemptIndex: number): Promise<void> {
    if (this.stopped || !this.browserosId) {
      return
    }

    try {
      const session = await this.connect({
        client: this.client,
        cache: this.cache,
        browserosId: this.browserosId,
        servers: getConnectorServerNames(),
      })
      if (this.stopped) {
        await session.close().catch((error) => {
          logger.warn('Failed to close Klavis proxy transport after stop', {
            error: error instanceof Error ? error.message : String(error),
          })
        })
        return
      }

      this.session = session
      this.status = { state: 'ready', toolCount: session.tools.length }
      logger.info('Klavis proxy connected', {
        attempt: attemptIndex + 1,
        toolCount: session.tools.length,
      })
    } catch (error) {
      if (this.stopped) {
        return
      }

      const msg = error instanceof Error ? error.message : String(error)
      if (attemptIndex < this.retryDelaysMs.length) {
        const delay = this.retryDelaysMs[attemptIndex]
        this.status = {
          state: 'retrying',
          attempt: attemptIndex + 1,
          nextRetryMs: delay,
          error: msg,
        }
        logger.info('Retrying Klavis proxy connection', {
          attempt: attemptIndex + 1,
          nextRetryMs: delay,
          error: msg,
        })
        this.retryTimer = setTimeout(() => {
          this.retryTimer = undefined
          if (!this.stopped) {
            this.status = { state: 'connecting' }
          }
          void this.attemptConnect(attemptIndex + 1)
        }, delay)
        return
      }

      this.status = { state: 'unavailable', error: msg }
      logger.warn(
        'Klavis proxy connection failed after all retries, MCP will serve browser tools only',
        { attempts: attemptIndex + 1, error: msg },
      )
    }
  }

  private toolAdapterDeps(scope: ConnectorToolScope) {
    return {
      catalog: this.listAvailableConnectors(),
      proxyStatus: this.getProxyStatus(),
      session: this.status.state === 'ready' ? this.session : null,
      scope,
      createConnectionIntent: (serverName: string) =>
        this.createConnectionIntent(serverName),
      getConnectorInventory: (inputScope?: ConnectorToolScope) =>
        this.getConnectorInventory(inputScope),
      getUserIntegrations: () => this.getUserIntegrations(),
    }
  }

  private requireBrowserosId(): string {
    if (!this.browserosId) {
      throw new Error('browserosId not configured')
    }
    return this.browserosId
  }

  private requireSupportedConnector(serverName: string): void {
    if (!isSupportedConnector(serverName)) {
      throw new Error(`Invalid server: ${serverName}`)
    }
  }
}
