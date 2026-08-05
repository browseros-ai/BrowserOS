/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AcpxMcpServerConfig } from '@browseros/acpx-ai-provider'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'

const BROWSEROS_MCP_NAME = 'browseros'

export interface BuildAcpMcpServersInput {
  serverPort: number
  conversationId: string
  browserContext?: BrowserContext
}

export function buildAcpMcpServers(
  input: BuildAcpMcpServersInput,
): AcpxMcpServerConfig[] {
  const headers: Record<string, string> = {
    'X-BrowserOS-Scope-Id': input.conversationId,
  }
  const browserContext = input.browserContext

  if (browserContext?.windowId !== undefined) {
    headers['X-BrowserOS-Default-Window-Id'] = String(browserContext.windowId)
  }
  if (browserContext?.enabledMcpServers?.length) {
    headers['X-BrowserOS-Managed-Mcp-Servers'] =
      browserContext.enabledMcpServers.map(encodeURIComponent).join(',')
  }

  const servers: AcpxMcpServerConfig[] = [
    {
      type: 'http',
      name: BROWSEROS_MCP_NAME,
      url: `http://127.0.0.1:${input.serverPort}/mcp`,
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
