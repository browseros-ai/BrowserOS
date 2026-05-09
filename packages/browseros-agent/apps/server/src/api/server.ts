/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Consolidated HTTP Server
 *
 * This server combines:
 * - Agent HTTP routes (chat, klavis, provider)
 * - MCP HTTP routes (using @hono/mcp transport)
 */

import { OPENCLAW_GATEWAY_CONTAINER_NAME } from '@browseros/shared/constants/openclaw'
import { Hono } from 'hono'
import { websocket } from 'hono/bun'
import { cors } from 'hono/cors'
import type { ContentfulStatusCode } from 'hono/utils/http-status'
import { HttpAgentError } from '../agent/errors'
import { INLINED_ENV } from '../env'
import { KlavisClient } from '../lib/clients/klavis/klavis-client'
import { initializeOAuth, shutdownOAuth } from '../lib/clients/oauth'
import { getDb } from '../lib/db'
import { logger } from '../lib/logger'
import { Sentry } from '../lib/sentry'
import { getLimaHomeDir, resolveBundledLimactl, VM_NAME } from '../lib/vm'
import { createAclRoutes } from './routes/acl'
import { createAgentRoutes } from './routes/agents'
import { createChatRoutes } from './routes/chat'
import { createContextRoutes } from './routes/context'
import { createCreditsRoutes } from './routes/credits'
import { createHealthRoute } from './routes/health'
import { createKlavisRoutes } from './routes/klavis'
import { createMcpRoutes } from './routes/mcp'
import { createMemoryRoutes } from './routes/memory'
import { createMonitoringRoutes } from './routes/monitoring'
import { createOAuthRoutes } from './routes/oauth'
import { createOpenClawRoutes } from './routes/openclaw'
import { createProviderRoutes } from './routes/provider'
import { createRefinePromptRoutes } from './routes/refine-prompt'
import { createShutdownRoute } from './routes/shutdown'
import { createSkillsRoutes } from './routes/skills'
import { createSoulRoutes } from './routes/soul'
import { createStatusRoute } from './routes/status'
import { createTerminalRoutes } from './routes/terminal'
import { GlobalAclPolicyService } from './services/acl/global-acl-policy'
import {
  connectKlavisInBackground,
  type KlavisProxyRef,
} from './services/klavis/strata-proxy'
import { convertOpenClawHistoryToAgentHistory } from './services/openclaw/history-mapper'
import { getOpenClawService } from './services/openclaw/openclaw-service'
import type { Env, HttpServerConfig } from './types'
import { defaultCorsConfig } from './utils/cors'
import { requireTrustedAppOrigin } from './utils/request-auth'

async function assertPortAvailable(port: number): Promise<void> {
  const net = await import('node:net')
  return new Promise((resolve, reject) => {
    const probe = net.createServer()

    probe.once('error', (err: NodeJS.ErrnoException) => {
      if (err.code === 'EADDRINUSE') {
        reject(
          Object.assign(new Error(`Port ${port} is already in use`), {
            code: 'EADDRINUSE',
          }),
        )
      } else {
        reject(err)
      }
    })

    probe.listen({ port, host: '127.0.0.1', exclusive: true }, () => {
      probe.close(() => resolve())
    })
  })
}

export async function createHttpServer(config: HttpServerConfig) {
  const {
    port,
    host = '0.0.0.0',
    browserosId,
    executionDir,
    resourcesDir,
    version,
    browser,
    registry,
  } = config

  const { onShutdown } = config
  const tokenManager = browserosId
    ? initializeOAuth(getDb(), browserosId)
    : null
  if (!browserosId) shutdownOAuth()

  const aclPolicyService = new GlobalAclPolicyService()
  await aclPolicyService.load()

  // Connect Klavis proxy in background with retry — browser tools available immediately
  const klavisRef: KlavisProxyRef = { handle: null }
  const stopKlavisBackground = browserosId
    ? connectKlavisInBackground(klavisRef, {
        klavisClient: new KlavisClient(),
        browserosId,
      })
    : () => {}

  const clawRoutes = new Hono<Env>()
    .use('/*', requireTrustedAppOrigin())
    .route('/', createOpenClawRoutes())

  const terminalRoutes = new Hono<Env>()
    .use('/*', requireTrustedAppOrigin())
    .route(
      '/',
      createTerminalRoutes({
        containerName: OPENCLAW_GATEWAY_CONTAINER_NAME,
        limaHome: getLimaHomeDir(),
        limactlPath: () => resolveBundledLimactl(resourcesDir),
        vmName: VM_NAME,
      }),
    )

  const aclRoutes = new Hono<Env>()
    .use('/*', requireTrustedAppOrigin())
    .route('/', createAclRoutes({ policyService: aclPolicyService }))

  const monitoringRoutes = new Hono<Env>()
    .use('/*', requireTrustedAppOrigin())
    .route('/', createMonitoringRoutes())

  const contextRoutes = new Hono<Env>()
    .use('/*', requireTrustedAppOrigin())
    .route('/', createContextRoutes())

  const agentRoutes = new Hono<Env>()
    .use('/*', requireTrustedAppOrigin())
    .route(
      '/',
      createAgentRoutes({
        browserosServerPort: port,
        browser,
        openclawGateway: {
          getContainerName: () => OPENCLAW_GATEWAY_CONTAINER_NAME,
          getLimaHomeDir: () => getLimaHomeDir(),
          getLimactlPath: () => resolveBundledLimactl(resourcesDir),
          getVmName: () => VM_NAME,
        },
        openclawProvisioner: {
          createAgent: (input) => getOpenClawService().createAgent(input),
          removeAgent: (agentId) => getOpenClawService().removeAgent(agentId),
          listAgents: async () => {
            const agents = await getOpenClawService().listAgents()
            return agents.map((agent) => ({
              agentId: agent.agentId,
              name: agent.name,
              model: agent.model,
            }))
          },
          getStatus: () => getOpenClawService().getStatus(),
          getAgentHistory: async (agentId) => {
            // Aggregated across the agent's main + every sub-session
            // (cron / hook / channel) so autonomous turns surface in
            // the chat panel alongside user-initiated ones.
            const raw = await getOpenClawService().getSessionHistory(
              `agent:${agentId}:main`,
            )
            return convertOpenClawHistoryToAgentHistory(agentId, raw)
          },
        },
        onTurnLifecycle: (agent, event) => {
          if (agent.adapter !== 'openclaw') return
          getOpenClawService().recordAgentTurnEvent(
            agent.id,
            agent.sessionKey,
            event,
          )
        },
      }),
    )

  const app = new Hono<Env>()
    .use('/*', cors(defaultCorsConfig))
    .route('/health', createHealthRoute({ browser }))
    .route(
      '/shutdown',
      createShutdownRoute({
        onShutdown: () => {
          shutdownOAuth()
          stopKlavisBackground()
          klavisRef.handle?.close().catch((err) =>
            logger.warn('Failed to close Klavis proxy transport', {
              error: err instanceof Error ? err.message : String(err),
            }),
          )
          onShutdown?.()
        },
      }),
    )
    .route('/status', createStatusRoute({ browser }))
    .route('/soul', createSoulRoutes())
    .route('/memory', createMemoryRoutes())
    .route('/context', contextRoutes)
    .route('/skills', createSkillsRoutes())
    .route('/monitoring', monitoringRoutes)
    .route('/acl-rules', aclRoutes)
    .route('/test-provider', createProviderRoutes({ browserosId }))
    .route('/refine-prompt', createRefinePromptRoutes({ browserosId }))
    .route(
      '/oauth',
      tokenManager
        ? createOAuthRoutes({ tokenManager })
        : new Hono().all('/*', (c) =>
            c.json({ error: 'OAuth not available' }, 503),
          ),
    )
    .route('/klavis', createKlavisRoutes({ browserosId: browserosId || '' }))
    .route(
      '/credits',
      createCreditsRoutes({
        browserosId,
        gatewayBaseUrl: INLINED_ENV.BROWSEROS_CONFIG_URL
          ? new URL(INLINED_ENV.BROWSEROS_CONFIG_URL).origin
          : undefined,
      }),
    )
    .route(
      '/mcp',
      createMcpRoutes({
        version,
        registry,
        browser,
        executionDir,
        resourcesDir,
        policyService: aclPolicyService,
        klavisRef,
      }),
    )
    .route(
      '/chat',
      createChatRoutes({
        browser,
        registry,
        browserosId,
        klavisRef,
        aiSdkDevtoolsEnabled: config.aiSdkDevtoolsEnabled,
      }),
    )
    .route('/agents', agentRoutes)
    .route('/claw', clawRoutes)

  // Error handler
  app.onError((err, c) => {
    const error = err as Error

    if (error instanceof HttpAgentError) {
      logger.warn('HTTP Agent Error', {
        name: error.name,
        message: error.message,
        code: error.code,
        statusCode: error.statusCode,
      })
      return c.json(error.toJSON(), error.statusCode as ContentfulStatusCode)
    }

    Sentry.withScope((scope) => {
      scope.setTag('route', c.req.path)
      scope.setTag('method', c.req.method)
      Sentry.captureException(error)
    })

    logger.error('Unhandled Error', {
      message: error.message,
      stack: error.stack,
    })

    return c.json(
      {
        error: {
          name: 'InternalServerError',
          message: error.message || 'An unexpected error occurred',
          code: 'INTERNAL_SERVER_ERROR',
          statusCode: 500,
        },
      },
      500,
    )
  })

  await assertPortAvailable(port)

  app.route('/terminal', terminalRoutes)

  const server = Bun.serve({
    fetch: (request, server) => app.fetch(request, { server }),
    port,
    hostname: host,
    idleTimeout: 0,
    websocket,
  })

  logger.info('Consolidated HTTP Server started', { port, host })

  if (config.aiSdkDevtoolsEnabled) {
    logger.info(
      'AI SDK DevTools enabled — run `npx @ai-sdk/devtools` to open the viewer',
    )
  }

  return {
    app,
    server,
    config,
  }
}
