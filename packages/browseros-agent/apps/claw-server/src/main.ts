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
import { migrateMcpConfigPaths } from './lib/migrate-mcp-config-paths'
import { migrateMcpUrls } from './lib/migrate-mcp-urls'
import { writeRuntimeFile } from './lib/runtime-file'
import { initializeTabTargets, stopTabTargets } from './lib/tab-targets'
import { setLocalServerUrl } from './local-server-url'
import { createServer } from './server'
import { captureEvent, shutdownAnalytics } from './services/analytics'
import { runIntegrityScan } from './services/integrity-scan'
import { recordingStore, startRecordingRetention } from './services/recordings'
import { releaseAllOpenSessionTabs } from './services/session-tabs'
import { releaseAllOpenClaims } from './services/tab-claims'
import { publicMcpUrl } from './shared/mcp-url'

async function start(): Promise<void> {
  const config = loadClawConfig()
  if (!config.ok) {
    // biome-ignore lint/suspicious/noConsole: pre-logger startup error
    console.error(config.error)
    process.exit(1)
  }
  applyClawConfig(config.value)

  releaseAllOpenClaims()
  releaseAllOpenSessionTabs()
  // Ingest clients drop unknown-tab batches, so seed identity before health can report ready.
  const bootstrap = await bootstrapBrowserosBrowser()
  if (bootstrap) {
    setBrowserSession(bootstrap.session)
    await initializeTabTargets(bootstrap.session)
  }

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
  const recordingRetention = startRecordingRetention(
    recordingStore,
    env.replayRetentionDays,
  )
  const url = `http://${httpServer.hostname}:${httpServer.port}`
  setLocalServerUrl(url)
  logger.info('claw-server listening', { url })
  // Anonymous active-install signal (one per boot). No url/port sent.
  captureEvent('server_started')
  // Publish the running URL to <CONFIG_DIR>/runtime.json so external
  // discovery (the Claude Desktop extension) can read the port without
  // scanning, log-tailing, or waiting for a harness link to populate
  // the mcp-manager manifest.
  //
  // Intentionally NOT awaited: writeRuntimeFile owns its own error
  // handling (logs a warning on failure, never throws). Awaiting here
  // would gate the integrity scan and MCP URL migration on a
  // best-effort disk write that can hang on a stalled network / FUSE /
  // container-mounted filesystem, even though the socket is already serving.
  void writeRuntimeFile(url)

  if (bootstrap) {
    logger.info('cockpit attached to browseros browser', {
      cdpPort: env.cdpPort,
    })
  }
  let exiting = false
  // Stop intake before draining writes so no claim or batch can arrive after closure.
  const cleanup = (): void => {
    if (exiting) return
    exiting = true
    const retentionDrain = recordingRetention.stop()
    const killSwitch = setTimeout(() => process.exit(1), 5_000)
    killSwitch.unref()
    void (async () => {
      await Promise.allSettled([httpServer.stop()])
      await retentionDrain
      await Promise.allSettled([recordingStore.close()])
      stopTabTargets()
      releaseAllOpenClaims()
      releaseAllOpenSessionTabs()
      const shutdownTasks: Promise<void>[] = [shutdownAnalytics()]
      if (bootstrap) shutdownTasks.push(bootstrap.disconnect())
      await Promise.allSettled(shutdownTasks)
    })().finally(() => {
      clearTimeout(killSwitch)
      process.exit(0)
    })
  }
  shutdown = cleanup
  process.once('SIGINT', cleanup)
  process.once('SIGTERM', cleanup)

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

  // Self-heal loop 2: rewrite the shared BrowserClaw MCP spec when
  // its URL no longer matches the current public URL (proxy or bind
  // port bump between runs).
  try {
    const migration = await migrateMcpUrls(publicMcpUrl())
    logger.info('mcpUrl migration finished', { ...migration })
  } catch (err) {
    logger.error('mcpUrl migration failed unexpectedly', {
      error: err instanceof Error ? err.message : String(err),
    })
  }

  // Self-heal loop 3: relocate BrowserClaw MCP entries when the
  // agent catalog's OS-resolved default config path has moved
  // between BrowserClaw versions (Antigravity 1.x -> 2.x moved
  // from `~/.gemini/antigravity/` to `~/.gemini/config/`; future
  // agent updates will do the same). Without this, existing
  // installs keep rewriting the OLD file while the harness reads
  // the NEW one, and users see a green "Configured" badge on a
  // broken connection.
  try {
    const pathMigration = await migrateMcpConfigPaths()
    logger.info('mcpConfigPath migration finished', { ...pathMigration })
  } catch (err) {
    logger.error('mcpConfigPath migration failed unexpectedly', {
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
