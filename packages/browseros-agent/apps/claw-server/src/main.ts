#!/usr/bin/env bun
/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Standalone BrowserClaw API entry point.
 *
 * Binds Hono on 127.0.0.1 and serves routes under `/cockpit`; the
 * claw-app extension can override the base URL with `?apiUrl=` or
 * `VITE_BROWSEROS_CLAW_API_URL` when dev-watch selects a random port.
 */

if (typeof Bun === 'undefined') {
  // biome-ignore lint/suspicious/noConsole: pre-logger bootstrap notice
  console.error(
    'claw-server requires the Bun runtime. Install Bun (https://bun.sh) and re-run with `bun src/main.ts`.',
  )
  process.exit(1)
}

import { Hono } from 'hono'
import { loadClawConfig } from './config'
import { applyClawConfig, env } from './env'
import { bootstrapBrowserosBrowser } from './lib/browser-bootstrap'
import { setBrowserSession } from './lib/browser-session'
import { logger } from './lib/logger'
import { migrateMcpUrls } from './lib/migrate-mcp-urls'
import { setLocalServerUrl } from './local-server-url'
import server from './server'
import { COCKPIT_MOUNT_PREFIX } from './shared/port'

async function start(): Promise<void> {
  const config = loadClawConfig()
  if (!config.ok) {
    // biome-ignore lint/suspicious/noConsole: pre-logger startup error
    console.error(config.error)
    process.exit(1)
  }
  applyClawConfig(config.value)

  const root = new Hono().route(COCKPIT_MOUNT_PREFIX, server)
  const httpServer = Bun.serve({
    hostname: '127.0.0.1',
    port: env.port,
    fetch: root.fetch,
  })
  const url = `http://${httpServer.hostname}:${httpServer.port}${COCKPIT_MOUNT_PREFIX}`
  setLocalServerUrl(url)
  logger.info('claw-server listening', { url })

  // Attach to the BrowserOS Chromium so MCP `tools/call` dispatches
  // hit a real browser. The bootstrap soft-fails when BrowserOS is
  // not reachable: the cockpit keeps serving the UI, profile CRUD,
  // harness installs, and `tools/list`, and `tools/call` continues
  // to short-circuit with the existing "session not connected"
  // wire shape until the user restarts the cockpit with BrowserOS
  // up. Reattach on transient drops is the CdpBackend's job (we
  // pass `exitOnReconnectFailure: false` so it does not kill the
  // process).
  const bootstrap = await bootstrapBrowserosBrowser()
  if (bootstrap) {
    setBrowserSession(bootstrap.session)
    logger.info('cockpit attached to browseros browser', {
      cdpPort: env.cdpPort,
    })
    // `exiting` guards against double-cleanup when a supervisor sends
    // SIGINT and SIGTERM back-to-back. `process.once` removes each
    // handler independently, so without the flag a SIGTERM that
    // arrives while the SIGINT cleanup is still in flight would
    // restart `disconnect()` on an already-closing CDP connection.
    // The kill switch guarantees forward progress: a hung
    // `cdp.disconnect()` (half-open socket, network stall) would
    // otherwise leave the process stuck because both handlers have
    // already been removed and only SIGKILL could recover it.
    let exiting = false
    const cleanup = (): void => {
      if (exiting) return
      exiting = true
      setTimeout(() => process.exit(1), 5_000).unref()
      bootstrap.disconnect().finally(() => process.exit(0))
    }
    process.once('SIGINT', cleanup)
    process.once('SIGTERM', cleanup)
  }

  // Sweep stored profiles so their harness install and mcpUrl match
  // the standalone server's current loopback URL.
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

void start()
