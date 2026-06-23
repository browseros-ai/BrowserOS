/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Hono } from 'hono'
import { cors } from 'hono/cors'
import type { OAuthTokenManager } from '../../lib/clients/oauth/token-manager'
import { requireTrustedOrigin } from '../middleware/require-trusted-origin'
import type { KlavisService } from '../services/klavis'
import type { RemoteHermesService } from '../services/remote-hermes/remote-hermes-service'
import type { Env, HttpServerConfig } from '../types'
import { defaultCorsConfig } from '../utils/cors'
import { requireTrustedAppOrigin } from '../utils/request-auth'
import { createAcpxProbeRoutes } from './acpx-probe'
import { createAgentRoutes } from './agents'
import { createChatRoutes } from './chat'
import { createCreditsRoutes } from './credits'
import { createHealthRoute } from './health'
import { createKlavisRoutes } from './klavis'
import { createMcpRoutes } from './mcp'
import { createMcpManagerRoutes } from './mcp-manager'
import { createOAuthRoutes } from './oauth'
import { createProviderRoutes } from './provider'
import { createRefinePromptRoutes } from './refine-prompt'
import { createRemoteHermesRoutes } from './remote-hermes'
import { createScreencastRoute } from './screencast'
import { createShutdownRoute } from './shutdown'
import { createStatusRoute } from './status'

interface CreateApiRoutesDeps {
  agentRoutes?: Hono<Env>
  config: HttpServerConfig
  gatewayBaseUrl?: string
  klavis: KlavisService
  onShutdown: () => void
  remoteHermes: RemoteHermesService | null
  tokenManager: OAuthTokenManager | null
}

/** Composes the BrowserOS HTTP API from the existing route factories. */
export function createApiRoutes(deps: CreateApiRoutesDeps) {
  const {
    agentRoutes,
    config,
    gatewayBaseUrl,
    klavis,
    remoteHermes,
    tokenManager,
  } = deps
  const {
    browser,
    browserosId,
    browserSession,
    executionDir,
    port,
    resourcesDir,
    version,
  } = config

  return new Hono<Env>()
    .use('/*', cors(defaultCorsConfig))
    .use('/*', requireTrustedOrigin())
    .route('/health', createHealthRoute({ browser }))
    .route('/shutdown', createShutdownRoute({ onShutdown: deps.onShutdown }))
    .route('/status', createStatusRoute({ browser }))
    .route(
      '/test-provider',
      createProviderRoutes({ browserosId, resourcesDir }),
    )
    .route('/acpx/probe', createAcpxProbeRoutes({ resourcesDir }))
    .route('/refine-prompt', createRefinePromptRoutes({ browserosId }))
    .route('/oauth', oauthRoutes(tokenManager))
    .route('/klavis', createKlavisRoutes({ klavis }))
    .route(
      '/credits',
      createCreditsRoutes({
        browserosId,
        gatewayBaseUrl,
      }),
    )
    .route(
      '/mcp',
      createMcpRoutes({
        version,
        browserSession,
        klavis,
        executionDir,
      }),
    )
    .route(
      '/mcp-manager',
      createMcpManagerRoutes({
        getMcpUrl: () => `http://127.0.0.1:${port}/mcp`,
      }),
    )
    .route(
      '/chat',
      createChatRoutes({
        browser,
        browserSession,
        browserosId,
        klavis,
        aiSdkDevtoolsEnabled: config.aiSdkDevtoolsEnabled,
        serverPort: port,
        resourcesDir,
        remoteHermes,
      }),
    )
    .route('/screencast', createScreencastRoute({ browser }))
    .route('/agents', protectedAgentRoutes(config, agentRoutes))
    .route(
      '/remote-hermes',
      createRemoteHermesRoutes({ service: remoteHermes }),
    )
}

function protectedAgentRoutes(config: HttpServerConfig, routes?: Hono<Env>) {
  return new Hono<Env>().use('/*', requireTrustedAppOrigin()).route(
    '/',
    routes ??
      createAgentRoutes({
        browserosServerPort: config.port,
        resourcesDir: config.resourcesDir,
        browser: config.browser,
      }),
  )
}

function oauthRoutes(tokenManager: OAuthTokenManager | null) {
  const app = new Hono<Env>()
  if (tokenManager) return app.route('/', createOAuthRoutes({ tokenManager }))

  return app.all('/*', (c) => c.json({ error: 'OAuth not available' }, 503))
}
