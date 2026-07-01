/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Single chokepoint for non-startup env reads. Startup config lives
 * in config.ts so it can be validated before serving.
 */

import type { ClawConfig } from './config'
import { resolveDefaultResourcesDir } from './config'
import { CLAW_API_PORT_DEFAULT, CLAW_CDP_PORT_DEFAULT } from './shared/port'

function readBrowserosDirOverride(): string | undefined {
  // biome-ignore lint/style/noProcessEnv: env.ts is the sanctioned env-reader for the package
  const raw = process.env.BROWSEROS_DIR?.trim()
  return raw && raw.length > 0 ? raw : undefined
}

function readIsDevelopment(): boolean {
  // biome-ignore lint/style/noProcessEnv: env.ts is the sanctioned env-reader for the package
  return process.env.NODE_ENV === 'development'
}

/**
 * Opt-in gate for legacy MCP surfaces. Defaults to `false` so the
 * legacy path is invisible unless explicitly requested.
 */
function readBoolFlag(name: string): boolean {
  // biome-ignore lint/style/noProcessEnv: env.ts is the sanctioned env-reader for the package
  const raw = process.env[name]
  if (raw === undefined) return false
  const normalised = raw.trim().toLowerCase()
  return normalised === '1' || normalised === 'true'
}

/**
 * Parse a positive integer ms override, falling back to the
 * provided default on any non-positive / non-finite input. Used
 * for runtime tunables (sweep interval, idle timeout) the operator
 * may want to shorten in dev or lengthen in staging.
 */
function readPositiveIntFlag(name: string, fallback: number): number {
  // biome-ignore lint/style/noProcessEnv: env.ts is the sanctioned env-reader for the package
  const raw = process.env[name]
  if (raw === undefined) return fallback
  const parsed = Number.parseInt(raw, 10)
  if (!Number.isFinite(parsed) || parsed <= 0) return fallback
  return parsed
}

/**
 * Opt-out gate for features that should be ON by default. Only `0`
 * and `false` (case-insensitive) disable; everything else including
 * an unset env keeps the feature on.
 */
function readBoolFlagDefaultTrue(name: string): boolean {
  // biome-ignore lint/style/noProcessEnv: env.ts is the sanctioned env-reader for the package
  const raw = process.env[name]
  if (raw === undefined) return true
  const normalised = raw.trim().toLowerCase()
  return normalised !== '0' && normalised !== 'false'
}

/**
 * Runtime snapshot shared across services. main.ts applies validated
 * startup config before serving; tests may mutate fields for isolation.
 */
export const env = {
  port: CLAW_API_PORT_DEFAULT,
  cdpPort: CLAW_CDP_PORT_DEFAULT,
  resourcesDir: resolveDefaultResourcesDir(),
  browserosDirOverride: readBrowserosDirOverride(),
  isDevelopment: readIsDevelopment(),
  // MCP session idle reaper. Sessions older than `sessionIdleMs`
  // with no inbound requests are torn down by the sweeper running
  // every `sessionSweepIntervalMs`. The 5-minute default matches
  // services/tasks.ts:IDLE_TIMEOUT_MS so the UI's status read and
  // the actual session-end row land at the same boundary.
  sessionIdleMs: readPositiveIntFlag('CLAW_SESSION_IDLE_MS', 5 * 60 * 1000),
  sessionSweepIntervalMs: readPositiveIntFlag(
    'CLAW_SESSION_SWEEP_INTERVAL_MS',
    60 * 1000,
  ),
  // Audit screenshot fallback. When on (default), a successful
  // state-mutating page-targeted dispatch that did not carry image
  // bytes in its result grabs the current screencast cache frame for
  // the tab and persists it as the dispatch's screenshot. Set to
  // `0`/`false` to revert to the strict behaviour where only the
  // explicit `screenshot` tool populates the audit's image column.
  screencastScreenshotFallback: readBoolFlagDefaultTrue(
    'CLAW_SCREENCAST_SCREENSHOT_FALLBACK',
  ),
}

/** Applies validated startup config to the shared runtime snapshot. */
export function applyClawConfig(config: ClawConfig): void {
  env.port = config.port
  env.cdpPort = config.cdpPort
  env.resourcesDir = config.resourcesDir
}

/**
 * Request-time read of the legacy per-slug MCP gate. Evaluated at
 * call time (not once at module load) so the existing per-slug
 * integration tests can flip the flag from `beforeAll` without
 * juggling import order. Default is `false`: the legacy URL shape
 * returns 404 unless the flag is explicitly set.
 */
export function isCockpitLegacyPerAgentMcpEnabled(): boolean {
  return readBoolFlag('COCKPIT_LEGACY_PER_AGENT_MCP')
}
