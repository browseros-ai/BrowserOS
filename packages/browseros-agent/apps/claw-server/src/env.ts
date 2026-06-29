/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Single chokepoint for non-port env reads. Port config lives in
 * config.ts so startup can validate env/YAML before serving.
 */

import type { ClawConfig } from './config'
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
 * Two opt-in escape hatches for legacy surfaces while the v2 cockpit
 * is the default. Both default to `false` so that the legacy is
 * invisible out of the box; setting either flag to `1` or `true`
 * brings the corresponding code path back.
 */
function readBoolFlag(name: string): boolean {
  // biome-ignore lint/style/noProcessEnv: env.ts is the sanctioned env-reader for the package
  const raw = process.env[name]
  if (raw === undefined) return false
  const normalised = raw.trim().toLowerCase()
  return normalised === '1' || normalised === 'true'
}

/**
 * Runtime snapshot shared across services. main.ts applies validated
 * port config before serving; tests may mutate fields for isolation.
 */
export const env = {
  port: CLAW_API_PORT_DEFAULT,
  cdpPort: CLAW_CDP_PORT_DEFAULT,
  browserosDirOverride: readBrowserosDirOverride(),
  isDevelopment: readIsDevelopment(),
}

/** Applies validated startup config to the shared runtime snapshot. */
export function applyClawConfig(config: ClawConfig): void {
  env.port = config.port
  env.cdpPort = config.cdpPort
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
