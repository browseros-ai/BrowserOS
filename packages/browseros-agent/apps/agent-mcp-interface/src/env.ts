/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Single chokepoint for env reads. Centralising here keeps the rest
 * of the source free of process.env access and lets biome's
 * noProcessEnv rule stay on at error level for every other file.
 */

import { COCKPIT_CDP_PORT_DEFAULT, PROD_API_PORT } from './shared/port'

function readPort(): number {
  // biome-ignore lint/style/noProcessEnv: env.ts is the sanctioned env-reader for the package
  const raw = process.env.BROWSEROS_AGENT_MCP_INTERFACE_PORT
  if (!raw) return PROD_API_PORT
  const parsed = Number(raw)
  if (!Number.isInteger(parsed) || parsed < 0 || parsed > 65535) {
    return PROD_API_PORT
  }
  return parsed
}

/**
 * Port the cockpit dials when attaching to the BrowserOS Chromium
 * over CDP. Default lives in IANA's dynamic / private range so it
 * cannot collide with a registered service; the env override is the
 * bridge until the BrowserOS browser shell defaults its DevTools
 * port to the same value.
 */
function readCdpPort(): number {
  // biome-ignore lint/style/noProcessEnv: env.ts is the sanctioned env-reader for the package
  const raw = process.env.BROWSEROS_COCKPIT_CDP_PORT
  if (!raw) return COCKPIT_CDP_PORT_DEFAULT
  const parsed = Number(raw)
  if (!Number.isInteger(parsed) || parsed < 1 || parsed > 65535) {
    return COCKPIT_CDP_PORT_DEFAULT
  }
  return parsed
}

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
 * Reads happen once at module load. Tests that need different values
 * mutate this object before importing the rest of the source graph;
 * production code treats it as immutable.
 */
export const env = {
  port: readPort(),
  cdpPort: readCdpPort(),
  browserosDirOverride: readBrowserosDirOverride(),
  isDevelopment: readIsDevelopment(),
}
