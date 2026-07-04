/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Browser-safe MCP URL constants shared by claw-server and claw-app.
 */

import { CLAW_API_PORT_DEFAULT } from './port'

export const MCP_PATH = '/mcp'
export const BROWSEROS_MCP_SERVER_NAME = 'BrowserClaw'
/** Names earlier BrowserOS/BrowserClaw eras wrote into harness configs. */
export const LEGACY_BROWSEROS_MCP_SERVER_NAMES = [
  'browseros',
  'browseros-stdio',
] as const

/** Builds the slugless local MCP URL shared by server config writers and UI copy helpers. */
export function canonicalMcpUrlForPort(port = CLAW_API_PORT_DEFAULT): string {
  return `http://127.0.0.1:${port}${MCP_PATH}`
}
