/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Hono application composition for the standalone BrowserClaw server.
 * Mounts the REST surface from the shared OpenAPI contract and the
 * protocol-level MCP endpoint. Callers create an isolated app instance
 * so tests can inject lifecycle hooks and HTTP handlers; the production
 * entry point owns shutdown behavior.
 */

import type { MiddlewareHandler } from 'hono'
import { Hono } from 'hono'
import { cors } from 'hono/cors'
import { createHttpRoute, type HttpHandlerSet } from './api/http'
import { canonicalApiError } from './api/http/errors'
import { productionConnectionHandlers } from './api/http/handlers/connections'
import { productionPreviewHandlers } from './api/http/handlers/previews'
import { productionRecordingHandlers } from './api/http/handlers/recordings'
import { productionReplayHandlers } from './api/http/handlers/replay'
import { productionScreenshotHandlers } from './api/http/handlers/screenshots'
import { productionSessionHandlers } from './api/http/handlers/sessions'
import { productionSettingsHandlers } from './api/http/handlers/settings'
import { createProductionSystemHandlers } from './api/http/handlers/system'
import { recordingIngestOriginHygiene } from './api/http/middleware/recording-origin'
import {
  type RequestContextEnv,
  requestIdFor,
  requestIdMiddleware,
} from './api/http/request-context'
import { mcpRoute } from './api/mcp'
import { logger } from './lib/logger'

// Telemetry capture is injectable so the server module stays usable
// from the bun-test runner without pulling Sentry into the import
// graph. main.ts can wire a real capture; tests get the no-op.
export type RouteErrorHandler = (
  err: unknown,
  path: string,
  method: string,
) => void

let captureRouteError: RouteErrorHandler = () => undefined

export function setRouteErrorHandler(fn: RouteErrorHandler): void {
  captureRouteError = fn
}

export const requestFailureLog: MiddlewareHandler = async (c, next) => {
  const start = Date.now()
  await next()
  const status = c.res.status
  if (status < 400) return
  const fields = {
    method: c.req.method,
    path: c.req.path,
    status,
    durationMs: Date.now() - start,
  }
  if (status >= 500) {
    logger.error('request failed', fields)
  } else {
    logger.warn('request failed', fields)
  }
}

interface CreateServerOptions {
  onShutdown?: () => void
  httpHandlers?: HttpHandlerSet
}

export function createServer(options: CreateServerOptions = {}) {
  const app = new Hono<RequestContextEnv>()

  app.use('*', requestIdMiddleware)
  app.use('/api/v1/recordings/events', recordingIngestOriginHygiene)
  // Read APIs remain available to the extension's opaque/null origin. The
  // write route above rejects web-page origins before this permissive CORS
  // layer can answer their preflight.
  app.use('*', cors({ origin: '*' }))

  // One structured line per failed request, however the failure was
  // produced: a router 404, a direct 4xx/5xx JSON return, or an
  // unhandled error — hono materialises the onError
  // response before `next()` resolves, so `c.res.status` is final
  // here. Sub-400 traffic stays unlogged on purpose: the logger has no
  // level filtering and the claw-app polls several endpoints, so a
  // per-request access log would flood the rotating log file.
  app.use('*', requestFailureLog)

  // Catch-all for genuinely unexpected errors. Routes today resolve
  // their own expected failures (404s, validation) inline and return
  // structured 4xx JSON. Anything that escapes that lands here, gets
  // reported via the injected capture, and turns into a structured 5xx
  // JSON body.
  app.onError((err, c) => {
    captureRouteError(err, c.req.path, c.req.method)
    // Contract routes must fail in-contract: the canonical `ApiError`
    // envelope with the request id, never the generic `{ error }` shape.
    // onError cannot tell which sub-router threw, so membership is by
    // path — `/api/v1/*` plus the two `/system` endpoints the contract
    // also owns.
    if (
      c.req.path.startsWith('/api/v1/') ||
      c.req.path === '/system/health' ||
      c.req.path === '/system/shutdown'
    ) {
      logger.error('Unhandled canonical route error', {
        path: c.req.path,
        method: c.req.method,
        error: err instanceof Error ? err.message : String(err),
      })
      return c.json(
        canonicalApiError(
          'internal_error',
          'internal server error',
          requestIdFor(c),
        ),
        500,
      )
    }
    const message = err instanceof Error ? err.message : 'internal error'
    logger.error('Unhandled route error', {
      path: c.req.path,
      method: c.req.method,
      error: message,
    })
    return c.json({ error: message }, 500)
  })

  const httpHandlers =
    options.httpHandlers ?? productionHttpHandlers(options.onShutdown)

  // The single MCP endpoint mounts at `/mcp`.
  return app.route('/', createHttpRoute(httpHandlers)).route('/', mcpRoute)
}

function productionHttpHandlers(onShutdown?: () => void): HttpHandlerSet {
  return {
    system: createProductionSystemHandlers(onShutdown),
    settings: productionSettingsHandlers,
    sessions: productionSessionHandlers,
    recordings: productionRecordingHandlers,
    replay: productionReplayHandlers,
    previews: productionPreviewHandlers,
    screenshots: productionScreenshotHandlers,
    connections: productionConnectionHandlers,
  }
}
