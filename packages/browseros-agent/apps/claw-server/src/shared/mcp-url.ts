/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Canonical v2 MCP URL shape. One endpoint per cockpit, no per-agent
 * slug. Used by both the server-side install service (writes the URL
 * into harness configs) and the UI's URL widgets (copy button + CLI
 * snippet). Keeping the shape in `shared/` mirrors the `shared/port`
 * precedent so a future cutover (e.g. a non-loopback bind) is a
 * single-file edit.
 */

import { CLAW_API_PORT_DEFAULT, COCKPIT_MOUNT_PREFIX } from './port'

/** Path the v2 single MCP route is mounted at, relative to the cockpit prefix. */
export const MCP_PATH = '/mcp'

/**
 * Server name written into harness configs. One canonical name across
 * every harness so the user sees "browseros" in every `claude mcp list`
 * / Cursor settings page. Same name reused by the canonical CLI snippet.
 */
export const BROWSEROS_MCP_SERVER_NAME = 'browseros'

/**
 * Builds the slugless canonical URL the v2 cockpit advertises. The
 * server side only ever uses 127.0.0.1; the UI's `mcp-endpoint.ts`
 * resolves alternate bases (dev-launcher overrides, query string).
 */
export function canonicalMcpUrlForPort(port = CLAW_API_PORT_DEFAULT): string {
  return `http://127.0.0.1:${port}${COCKPIT_MOUNT_PREFIX}${MCP_PATH}`
}
