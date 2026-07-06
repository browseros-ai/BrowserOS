/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Hono application composition. The chained `.route('/', xxxRoute)`
 * calls give us a `routes` reference whose inferred type captures
 * every endpoint's input / output shape; we re-export that as
 * `AppType` so the future claw-app can build a fully typed
 * hono-rpc client with `hc<AppType>(baseUrl)`.
 *
 * Bun + loopback-only bind; the chain shape is the standard hono-rpc
 * recipe.
 */

import type { MiddlewareHandler } from 'hono'
import { Hono } from 'hono'
import { cors } from 'hono/cors'
import { HttpError } from './lib/errors'
import { logger } from './lib/logger'
import { agentsControlRoute } from './routes/agents-control'
import { auditRoute } from './routes/audit'
import { auditScreenshotsRoute } from './routes/audit/screenshots'
import { auditTasksRoute } from './routes/audit/tasks'
import { auditReplayRoute } from './routes/audit-replay'
import { replayTabsRoute } from './routes/audit-replay/tabs'
import { connectionsRoute } from './routes/connections'
import { mcpV2Route } from './routes/mcp-v2'
import { createSystemRoute } from './routes/system'
import { tabsRoute } from './routes/tabs'

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
}

export function createServer(options: CreateServerOptions = {}) {
  const app = new Hono()

  // Loopback-only bind (see main.ts) makes wildcard CORS safe and
  // dodges the `null` Origin a chrome-extension:// page sends when
  // fetching from `http://127.0.0.1:<port>`.
  app.use('*', cors({ origin: '*' }))

  // One structured line per failed request, however the failure was
  // produced: a router 404, a thrown HttpError, a direct 4xx/5xx JSON
  // return, or an unhandled error — hono materialises the onError
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
    if (err instanceof HttpError) {
      return c.json({ error: err.message }, err.status as 400 | 404 | 409 | 500)
    }
    const message = err instanceof Error ? err.message : 'internal error'
    logger.error('Unhandled route error', {
      path: c.req.path,
      method: c.req.method,
      error: message,
    })
    return c.json({ error: message }, 500)
  })

  // The single MCP endpoint mounts at `/mcp`.
  return app
    .route('/', createSystemRoute({ onShutdown: options.onShutdown }))
    .route('/', mcpV2Route)
    .route('/', tabsRoute)
    .route('/', agentsControlRoute)
    .route('/', connectionsRoute)
    .route('/', auditRoute)
    .route('/', auditTasksRoute)
    .route('/', auditScreenshotsRoute)
    .route('/', auditReplayRoute)
    .route('/', replayTabsRoute)
}

const routes = createServer()

export type AppType = typeof routes
export default routes
