#!/usr/bin/env bun
/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Bun entry point for the agent-mcp-interface server.
 *
 * Binds Hono on 127.0.0.1 — same posture as @browseros/server. The
 * loopback restriction is what lets us run with wildcard CORS and
 * accept `null` Origin requests from the future WXT extension
 * loading via chrome-extension://. No external network reachability.
 *
 * Routes are mounted under `/cockpit` so the URL shape matches what
 * `createCockpitRoutes` produces when the cockpit is embedded inside
 * `@browseros/server`'s runtime (which is the production path). The
 * UI client and agent-mcp-manager harness configs use a single base
 * URL shape (`http://127.0.0.1:<port>/cockpit/...`) regardless of
 * which runtime is hosting them, so a profile created against
 * standalone keeps working when the user later switches to the
 * merged runtime on the same port (and vice versa).
 *
 * The agent-mcp-ui extension reads PROD_API_PORT off the shared port
 * constant; in dev it can pick up an `?apiUrl=` override published
 * by whichever launcher started this process.
 */

if (typeof Bun === 'undefined') {
  // biome-ignore lint/suspicious/noConsole: pre-logger bootstrap notice
  console.error(
    'agent-mcp-interface requires the Bun runtime. Install Bun (https://bun.sh) and re-run with `bun src/main.ts`.',
  )
  process.exit(1)
}

import { Hono } from 'hono'
import { env } from './env'
import { logger } from './lib/logger'
import { migrateMcpUrls } from './lib/migrate-mcp-urls'
import { setLocalServerUrl } from './local-server-url'
import server from './server'
import { COCKPIT_MOUNT_PREFIX } from './shared/port'

function start(): void {
  const root = new Hono().route(COCKPIT_MOUNT_PREFIX, server)
  const httpServer = Bun.serve({
    hostname: '127.0.0.1',
    port: env.port,
    fetch: root.fetch,
  })
  const url = `http://${httpServer.hostname}:${httpServer.port}${COCKPIT_MOUNT_PREFIX}`
  setLocalServerUrl(url)
  logger.info('agent-mcp-interface listening', { url })

  // Mirror what createCockpitRoutes does in the merged runtime: sweep
  // every stored profile and rewrite its harness install + mcpUrl to
  // the new `/cockpit`-prefixed shape if it carried the pre-merge
  // URL. Idempotent — a second run is a no-op once every profile is
  // up to date. The factory in the production path runs the same
  // sweep at boot.
  const buildMcpUrlForMigration = (slug: string): string => `${url}/mcp/${slug}`
  void migrateMcpUrls(buildMcpUrlForMigration)
    .then((result) =>
      logger.info('mcpUrl migration finished', {
        migrated: result.migrated,
        skipped: result.skipped,
        failed: result.failed,
      }),
    )
    .catch((err: unknown) =>
      logger.error('mcpUrl migration failed unexpectedly', {
        error: err instanceof Error ? err.message : String(err),
      }),
    )
}

start()
