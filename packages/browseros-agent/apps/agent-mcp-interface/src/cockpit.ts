/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The cockpit's "mount me inside apps/server" entry point.
 *
 * `createCockpitRoutes` is the function `apps/server`'s
 * `createHttpServer` calls to splice the cockpit's `/agents`,
 * `/site-rules`, `/permissions`, `/mcp/:slug`, and `/system` routes
 * into the parent Hono runtime under a `/cockpit` prefix. The
 * factory:
 *
 *   1. Wires the cockpit's process-wide singletons (the live
 *      `BrowserSession` for tool dispatch, and the base URL that
 *      `buildMcpUrl` uses to compose the per-agent endpoints).
 *   2. Runs a one-shot `migrateMcpUrls` sweep over the profile
 *      directory so any URLs saved before the runtime merge get
 *      rewritten to the new shape; harness configs get re-installed
 *      automatically as part of the migration.
 *   3. Returns the cockpit's Hono `app` so the parent can mount it
 *      via `.route('/cockpit', cockpitApp)`.
 *
 * The standalone `src/main.ts` still works for tests + solo dev
 * (the BrowserSession stays null in that path, so tool dispatches
 * short-circuit with "session not connected"). Production goes
 * through `createCockpitRoutes`.
 */

import type { BrowserSession } from '@browseros/server/browser/core/session'
import { setBrowserSession } from './lib/browser-session'
import { logger } from './lib/logger'
import { migrateMcpUrls } from './lib/migrate-mcp-urls'
import { setLocalServerUrl } from './local-server-url'
import app from './server'

export interface CockpitDeps {
  /** Live BrowserSession from the parent runtime. Tool dispatches bind to this. */
  browserSession: BrowserSession
  /** The port `apps/server` bound to; used to compose MCP URLs the harness can reach. */
  serverPort: number
  /**
   * Mount prefix the cockpit's app is being routed under. Embedded in
   * `buildMcpUrl` so per-agent URLs match the actual routable shape.
   * Defaults to `/cockpit`; passing `''` removes the prefix entirely
   * for testing.
   */
  mountPrefix?: string
}

export function createCockpitRoutes(deps: CockpitDeps): typeof app {
  const prefix = deps.mountPrefix ?? '/cockpit'
  setBrowserSession(deps.browserSession)
  setLocalServerUrl(`http://127.0.0.1:${deps.serverPort}${prefix}`)
  // Fire the migration in the background; one bad profile must not
  // block server startup. Result is logged so an operator can see
  // how many rows moved.
  void migrateMcpUrls(buildMcpUrlFromPort(deps.serverPort, prefix))
    .then((result) =>
      logger.info('mcpUrl migration finished', {
        migrated: result.migrated,
        skipped: result.skipped,
        failed: result.failed,
      }),
    )
    .catch((err: unknown) =>
      // `migrateMcpUrls` guards per-profile errors, but `listFiles` at its
      // start can still throw (EACCES, ENOTDIR, etc.). Without this `.catch`
      // the rejection lands as an unhandled promise rejection that Bun
      // discards silently. Logging keeps the failure visible to operators
      // without preventing the rest of the cockpit from coming up.
      logger.error('mcpUrl migration failed unexpectedly', {
        error: err instanceof Error ? err.message : String(err),
      }),
    )
  return app
}

function buildMcpUrlFromPort(
  serverPort: number,
  prefix: string,
): (slug: string) => string {
  return (slug: string) => `http://127.0.0.1:${serverPort}${prefix}/mcp/${slug}`
}
