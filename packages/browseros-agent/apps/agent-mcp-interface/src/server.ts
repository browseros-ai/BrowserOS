/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Hono application composition. The chained `.route('/', xxxRoute)`
 * calls give us a `routes` reference whose inferred type captures
 * every endpoint's input / output shape; we re-export that as
 * `AppType` so the future agent-mcp-ui can build a fully typed
 * hono-rpc client with `hc<AppType>(baseUrl)`.
 *
 * Bun + loopback-only bind; the chain shape is the standard hono-rpc
 * recipe.
 */

import { Hono } from 'hono'
import { cors } from 'hono/cors'
import { HttpError } from './lib/errors'
import { logger } from './lib/logger'
import { agentsRoute } from './routes/agents'
import { connectionsRoute } from './routes/connections'
import { mcpRoute } from './routes/mcp'
import { mcpV2Route } from './routes/mcp-v2'
import { permissionsRoute } from './routes/permissions'
import { siteRulesRoute } from './routes/site-rules'
import { systemRoute } from './routes/system'
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

const app = new Hono()

// Loopback-only bind (see main.ts) makes wildcard CORS safe and
// dodges the `null` Origin a chrome-extension:// page sends when
// fetching from `http://127.0.0.1:<port>`.
app.use('*', cors({ origin: '*' }))

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

// v2 single-MCP route always mounts at `/mcp`. The legacy per-slug
// route at `/mcp/:slug` also mounts always, but its handler returns
// 404 unless `COCKPIT_LEGACY_PER_AGENT_MCP=1` is set; the gate runs
// per-request so tests can flip it from `beforeAll` without juggling
// import order.
const routes = app
  .route('/', systemRoute)
  .route('/', agentsRoute)
  .route('/', siteRulesRoute)
  .route('/', permissionsRoute)
  .route('/', mcpV2Route)
  .route('/', mcpRoute)
  .route('/', tabsRoute)
  .route('/', connectionsRoute)

export type AppType = typeof routes
export default routes
