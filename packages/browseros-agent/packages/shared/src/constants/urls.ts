/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { CLAW_API_PORT_DEFAULT } from './ports'

export const MCP_PATH = '/mcp'
export const BROWSEROS_MCP_SERVER_NAME = 'browseros-neo'

export function canonicalMcpUrlForPort(port = CLAW_API_PORT_DEFAULT): string {
  return `http://127.0.0.1:${port}${MCP_PATH}`
}

export const EXTERNAL_URLS = {
  CDN: 'https://cdn.browseros.com',
} as const
