/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Hono } from 'hono'
import { cors } from 'hono/cors'
import { AcpAgentRuntime } from '../../lib/agents/acp/acp-agent-runtime'
import type { OAuthTokenManager } from '../../lib/clients/oauth/token-manager'
import { requireTrustedOrigin } from '../middleware/require-trusted-origin'
import type { KlavisService } from '../services/klavis'
import type { Env, HttpServerConfig } from '../types'
import { defaultCorsConfig } from '../utils/cors'
import { requireTrustedAppOrigin } from '../utils/request-auth'
import { createAcpxProbeRoutes } from './acpx-probe'
import { createAgentRoutes } from './agents'
import { createChatRoutes } from './chat'
import { createConversationRoutes } from './conversations'
import { createCreditsRoutes } from './credits'
import { createHealthRoute } from './health'
import { createKlavisRoutes } from './klavis'
import { createMcpRoutes } from './mcp'
import { createMcpManagerRoutes } from './mcp-manager'
import { createOAuthRoutes } from './oauth'
import { createProviderRoutes } from './provider'
import { createRefinePromptRoutes } from './refine-prompt'
import { createShutdownRoute } from './shutdown'
import { createStatusRoute } from './status'

interface CreateApiRoutesDeps {
  agentRoutes?: Hono<Env>
  config: HttpServerConfig
  gatewayBaseUrl?: string
  klavis: KlavisService
  onShutdown: () => void
  tokenManager: OAuthTokenManager | null
}

/** Composes the BrowserOS HTTP API from the existing route factories. */
export function createApiRoutes(deps: CreateApiRoutesDeps) {
  const {
    agentRoutes,
    config,
    gatewayBaseUrl,
    klavis,
    onShutdown,
    tokenManager,
  } = deps
  const { browser, browserosId, browserSession, port, resourcesDir, version } =
    config
  const { activity } = config
  const acpRuntime = new AcpAgentRuntime({ serverPort: port, resourcesDir })
  const resolvedAgentRoutes =
    agentRoutes ??
    createAgentRoutes({
      onDelete: (agentId) =>
        acpRuntime.closeAllForAgent(agentId, {
          discardPersistentState: true,
        }),
    })

  return (
    new Hono<Env>()
      .use('/*', cors(defaultCorsConfig))
      .use('/*', requireTrustedOrigin())
      .route('/system/health', createHealthRoute({ browser }))
      .route('/system/shutdown', createShutdownRoute({ onShutdown }))
      // Compatibility aliases for shipped browsers that still probe root paths
      // while the server binary can update independently during OTA.
      .route('/health', createHealthRoute({ browser }))
      .route('/shutdown', createShutdownRoute({ onShutdown }))
      .route('/status', createStatusRoute({ browser, activity }))
      .route('/test-provider', createProviderRoutes({ browserosId }))
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
          activity,
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
          activity,
          acpRuntime,
        }),
      )
      // Protected routes. The extension-origin auth middleware is applied per
      // path prefix (Hono's `/prefix/*` also matches the bare list route), so
      // unregistered paths stay 404 and public routes are untouched. Routes are
      // mounted directly; the typed client derives per-route types from the
      // factories (see rpc.ts), not from this composition.
      .use('/acpx/probe/*', requireTrustedAppOrigin())
      .use('/agents/*', requireTrustedAppOrigin())
      .use('/conversations/*', requireTrustedAppOrigin())
      .route('/acpx/probe', createAcpxProbeRoutes({ resourcesDir }))
      .route('/agents', resolvedAgentRoutes)
      .route('/conversations', createConversationRoutes())
  )
}

function oauthRoutes(tokenManager: OAuthTokenManager | null) {
  const app = new Hono<Env>()
  if (tokenManager) return app.route('/', createOAuthRoutes({ tokenManager }))

  return app.all('/*', (c) => c.json({ error: 'OAuth not available' }, 503))
}
