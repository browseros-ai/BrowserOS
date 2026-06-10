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
import { requireTrustedOrigin } from './middleware/require-trusted-origin'
import { createAcpxProbeRoutes } from './routes/acpx-probe'
import { createAgentRoutes } from './routes/agents'
import { createChatRoutes } from './routes/chat'
import { createCreditsRoutes } from './routes/credits'
import { createHealthRoute } from './routes/health'
import { createKlavisRoutes } from './routes/klavis'
import { createMcpRoutes } from './routes/mcp'
import { createOAuthRoutes } from './routes/oauth'
import { createProviderRoutes } from './routes/provider'
import { createRefinePromptRoutes } from './routes/refine-prompt'
import { createScreencastRoute } from './routes/screencast'
import { createShutdownRoute } from './routes/shutdown'
import { createStatusRoute } from './routes/status'
import {
  connectKlavisInBackground,
  type KlavisProxyRef,
} from './services/klavis/strata-proxy'
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
    resourcesDir,
    version,
    browser,
    browserSession,
  } = config

  const { onShutdown } = config
  const tokenManager = browserosId
    ? initializeOAuth(getDb(), browserosId)
    : null
  if (!browserosId) shutdownOAuth()

  // Connect Klavis proxy in background with retry — browser tools available immediately
  const klavisRef: KlavisProxyRef = { handle: null }
  const stopKlavisBackground = browserosId
    ? connectKlavisInBackground(klavisRef, {
        klavisClient: new KlavisClient(),
        browserosId,
      })
    : () => {}

  const agentRoutes = new Hono<Env>()
    .use('/*', requireTrustedAppOrigin())
    .route(
      '/',
      createAgentRoutes({
        browserosServerPort: port,
        resourcesDir,
        browser,
      }),
    )

  const app = new Hono<Env>()
    .use('/*', cors(defaultCorsConfig))
    .use('/*', requireTrustedOrigin())
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
    .route('/test-provider', createProviderRoutes({ browserosId }))
    .route('/acpx/probe', createAcpxProbeRoutes())
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
        browser,
        browserSession,
        klavisRef,
        browserUseNewTools: config.browserUseNewTools,
      }),
    )
    .route(
      '/chat',
      createChatRoutes({
        browser,
        browserSession,
        browserosId,
        klavisRef,
        aiSdkDevtoolsEnabled: config.aiSdkDevtoolsEnabled,
        serverPort: port,
      }),
    )
    .route('/screencast', createScreencastRoute({ browser }))
    .route('/agents', agentRoutes)

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
