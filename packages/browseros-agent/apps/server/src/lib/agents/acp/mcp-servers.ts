/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AcpxMcpServerConfig } from '@browseros/acpx-ai-provider'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import { BROWSEROS_TOOL_LEASE_HEADER } from '../../browser-tool-lease'

const BROWSEROS_MCP_NAME = 'browseros'

export interface BuildAcpMcpServersInput {
  serverPort: number
  browserToolLeaseToken: string
  readOnly: boolean
  browserContext?: BrowserContext
}

export function buildAcpMcpServers(
  input: BuildAcpMcpServersInput,
): AcpxMcpServerConfig[] {
  const headers: Record<string, string> = {
    [BROWSEROS_TOOL_LEASE_HEADER]: input.browserToolLeaseToken,
  }
  const browserContext = input.browserContext
  const readOnlyQuery = input.readOnly ? '?read_only=1' : ''

  const servers: AcpxMcpServerConfig[] = [
    {
      type: 'http',
      name: BROWSEROS_MCP_NAME,
      url: `http://127.0.0.1:${input.serverPort}/mcp${readOnlyQuery}`,
      headers,
    },
  ]

  for (const server of browserContext?.customMcpServers ?? []) {
    if (server.name.trim().toLowerCase() === BROWSEROS_MCP_NAME) continue
    servers.push({
      type: 'http',
      name: server.name,
      url: server.url,
      headers: {},
    })
  }

  return servers
}
