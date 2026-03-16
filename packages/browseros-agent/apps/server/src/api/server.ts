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
import { cors } from 'hono/cors'
import type { ContentfulStatusCode } from 'hono/utils/http-status'
import { HttpAgentError } from '../agent/errors'
import { KlavisClient } from '../lib/clients/klavis/klavis-client'
import { logger } from '../lib/logger'
import { createChatRoutes } from './routes/chat'
import { createGraphRoutes } from './routes/graph'
import { createHealthRoute } from './routes/health'
import { createKlavisRoutes } from './routes/klavis'
import { createMcpRoutes } from './routes/mcp'
import { createMemoryRoutes } from './routes/memory'
import { createProviderRoutes } from './routes/provider'
import { createSdkRoutes } from './routes/sdk'
import { createShutdownRoute } from './routes/shutdown'
import { createSkillsRoutes } from './routes/skills'
import { createSoulRoutes } from './routes/soul'
import { createStatusRoute } from './routes/status'
import {
  connectKlavisProxy,
  type KlavisProxyHandle,
} from './services/mcp/register-klavis-mcp'
import type { Env, HttpServerConfig } from './types'
import { defaultCorsConfig } from './utils/cors'

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
    rateLimiter,
    version,
    browser,
    controller,
    registry,
  } = config

  const { onShutdown } = config

  // Connect Klavis proxy (non-blocking: browser tools still work if this fails)
  let klavisProxy: KlavisProxyHandle | null = null
  if (browserosId) {
    try {
      klavisProxy = await connectKlavisProxy({
        klavisClient: new KlavisClient(),
        browserosId,
      })
    } catch (error) {
      logger.warn(
        'Failed to connect Klavis proxy, MCP will serve browser tools only',
        {
          error: error instanceof Error ? error.message : String(error),
        },
      )
    }
  }

  const app = new Hono<Env>()
    .use('/*', cors(defaultCorsConfig))
    .route('/health', createHealthRoute({ browser }))
    .route(
      '/shutdown',
      createShutdownRoute({
        onShutdown: () => {
          klavisProxy?.close().catch((err) =>
            logger.warn('Failed to close Klavis proxy transport', {
              error: err instanceof Error ? err.message : String(err),
            }),
          )
          onShutdown?.()
        },
      }),
    )
    .route('/status', createStatusRoute({ controller }))
    .route('/soul', createSoulRoutes())
    .route('/memory', createMemoryRoutes())
    .route('/skills', createSkillsRoutes())
    .route('/test-provider', createProviderRoutes())
    .route('/klavis', createKlavisRoutes({ browserosId: browserosId || '' }))
    .route(
      '/mcp',
      createMcpRoutes({
        version,
        registry,
        browser,
        executionDir,
        resourcesDir,
        klavisProxy,
      }),
    )
    .route(
      '/chat',
      createChatRoutes({
        browser,
        registry,
        browserosId,
        rateLimiter,
        aiSdkDevtoolsEnabled: config.aiSdkDevtoolsEnabled,
      }),
    )
    .route(
      '/sdk',
      createSdkRoutes({
        port,
        browser,
        browserosId,
      }),
    )
    .route(
      '/graph',
      createGraphRoutes({
        port,
        tempDir: executionDir,
        codegenServiceUrl: config.codegenServiceUrl,
      }),
    )

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
