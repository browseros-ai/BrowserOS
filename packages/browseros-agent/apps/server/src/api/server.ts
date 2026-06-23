/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { websocket } from 'hono/bun'
import type { ContentfulStatusCode } from 'hono/utils/http-status'
import { HttpAgentError } from '../agent/errors'
import { INLINED_ENV } from '../env'
import { TurnRegistry } from '../lib/agents/turns/active-turn-registry'
import { initializeOAuth, shutdownOAuth } from '../lib/clients/oauth'
import { RemoteHermesClient } from '../lib/clients/remote-hermes/remote-hermes-client'
import { getDb } from '../lib/db'
import { logger } from '../lib/logger'
import { Sentry } from '../lib/sentry'
import { createApiRoutes } from './routes'
import { REMOTE_AGENT_HARNESS_MCP_SOURCE } from './routes/mcp'
import { KlavisService } from './services/klavis'
import { RemoteHermesService } from './services/remote-hermes/remote-hermes-service'
import type { HttpServerConfig } from './types'

/** Checks the loopback bind before Bun.serve so startup errors stay explicit. */
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

/** Creates the Hono app and Bun server after wiring process-level dependencies. */
export async function createHttpServer(config: HttpServerConfig) {
  const { port, host = '0.0.0.0', browserosId } = config

  const { onShutdown } = config
  const tokenManager = browserosId
    ? initializeOAuth(getDb(), browserosId)
    : null
  if (!browserosId) shutdownOAuth()

  const klavis = new KlavisService({ browserosId })
  klavis.start()

  // Shared between createAgentRoutes (which owns the lifecycle) and
  // the nudge MCP route (which needs to push app_connection_request
  // events into the same active turns). Hoisting here means both
  // mounts hold the same instance.
  const turnRegistry = new TurnRegistry()

  // Remote Hermes provider. Opt-in via AGENT_RUNNER_JWT_SECRET in env;
  // when absent we still wire the routes but they return a soft
  // not_configured response (agent UI degrades gracefully).
  const remoteHermes =
    browserosId && INLINED_ENV.AGENT_RUNNER_JWT_SECRET
      ? new RemoteHermesService({
          client: new RemoteHermesClient({
            browserosId,
            jwtSecret: INLINED_ENV.AGENT_RUNNER_JWT_SECRET,
          }),
          resolveLocalMcpUrl: (server) =>
            server === 'browseros'
              ? `http://127.0.0.1:${port}/mcp?source=${REMOTE_AGENT_HARNESS_MCP_SOURCE}`
              : null,
        })
      : null
  if (!remoteHermes) {
    logger.warn('Remote Hermes disabled: AGENT_RUNNER_JWT_SECRET not set')
  }

  const app = createApiRoutes({
    config,
    gatewayBaseUrl: INLINED_ENV.BROWSEROS_CONFIG_URL
      ? new URL(INLINED_ENV.BROWSEROS_CONFIG_URL).origin
      : undefined,
    klavis,
    remoteHermes,
    tokenManager,
    turnRegistry,
    onShutdown: () => {
      shutdownOAuth()
      void klavis.stop()
      remoteHermes?.close()
      onShutdown?.()
    },
  })

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
