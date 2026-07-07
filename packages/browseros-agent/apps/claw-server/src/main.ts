#!/usr/bin/env bun
/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Standalone BrowserClaw API entry point.
 *
 * Binds Hono on 127.0.0.1 and serves routes from the root; the
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

import { loadClawConfig } from './config'
import { applyClawConfig, env } from './env'
import { bootstrapBrowserosBrowser } from './lib/browser-bootstrap'
import { setBrowserSession } from './lib/browser-session'
import { getClawServerDir } from './lib/browserclaw-dir'
import { logger } from './lib/logger'
import { migrateMcpUrls } from './lib/migrate-mcp-urls'
import { setLocalServerUrl } from './local-server-url'
import { createServer } from './server'
import { runIntegrityScan } from './services/integrity-scan'
import { startScreencastPoller } from './services/screencast-poller'
import { publicMcpUrl } from './shared/mcp-url'

async function start(): Promise<void> {
  const config = loadClawConfig()
  if (!config.ok) {
    // biome-ignore lint/suspicious/noConsole: pre-logger startup error
    console.error(config.error)
    process.exit(1)
  }
  applyClawConfig(config.value)

  let shutdown = (): void => process.exit(0)
  const app = createServer({ onShutdown: () => shutdown() })
  const httpServer = Bun.serve({
    hostname: '127.0.0.1',
    port: env.serverPort,
    fetch: app.fetch,
  })
  // File sink attaches only after the port bind succeeds: the bind is
  // the de-facto singleton lock, so a second accidental launch dies on
  // EADDRINUSE before it can rotate the live instance's log file.
  logger.setLogFile(getClawServerDir())
  const url = `http://${httpServer.hostname}:${httpServer.port}`
  setLocalServerUrl(url)
  logger.info('claw-server listening', { url })

  // Self-heal loop 1: diff manifest vs. on-disk agent configs and
  // relink any drifted / missing entries from the manifest-stored
  // spec. Fires before the URL migration so relinks use the current
  // spec URL; the migration then overwrites URLs that have moved.
  try {
    const scan = await runIntegrityScan()
    logger.info('integrity scan finished', { ...scan })
  } catch (err) {
    logger.warn('integrity scan failed unexpectedly', {
      error: err instanceof Error ? err.message : String(err),
    })
  }

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
    // Drive the Running-now homepage screencast against the live
    // session. Cleanly stopped on SIGINT/SIGTERM below; the handle is
    // a no-op interval (unref'd) so it never blocks shutdown.
    const screencast = startScreencastPoller({ session: bootstrap.session })
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
      screencast.stop()
      setTimeout(() => process.exit(1), 5_000).unref()
      bootstrap.disconnect().finally(() => process.exit(0))
    }
    shutdown = cleanup
    process.once('SIGINT', cleanup)
    process.once('SIGTERM', cleanup)
  }

  // Self-heal loop 2: rewrite every managed MCP spec whose URL no
  // longer matches the current public URL (proxy or bind port bump
  // between runs) and update stored profile JSON to match. Covers
  // per-profile installs AND the shared BrowserClaw entry.
  try {
    const migration = await migrateMcpUrls(publicMcpUrl())
    logger.info('mcpUrl migration finished', { ...migration })
  } catch (err) {
    logger.error('mcpUrl migration failed unexpectedly', {
      error: err instanceof Error ? err.message : String(err),
    })
  }
}

start().catch((error: unknown) => {
  logger.error('claw-server startup failed', {
    error: error instanceof Error ? error.message : String(error),
    stack: error instanceof Error ? error.stack : undefined,
  })
  process.exit(1)
})
