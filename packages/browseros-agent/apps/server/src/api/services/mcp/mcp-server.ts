/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { SetLevelRequestSchema } from '@modelcontextprotocol/sdk/types.js'
import type { BrowserSession } from '../../../browser/core/session'
import type { ConnectorToolScope, KlavisService } from '../klavis'
import { MCP_INSTRUCTIONS } from './mcp-prompt'
import { type RemoteAgentHarnessTools, registerTools } from './register-mcp'

export interface McpServiceDeps {
  version: string
  browserSession: BrowserSession
  klavis?: KlavisService
  connectorScope?: ConnectorToolScope
  defaultWindowId?: number
  defaultTabGroupId?: string
  executionDir: string
  remoteAgentHarness?: RemoteAgentHarnessTools
}

/** Creates a per-request BrowserOS MCP server with tools for the requested surface. */
export function createMcpServer(deps: McpServiceDeps): McpServer {
  const server = new McpServer(
    {
      name: 'browseros_mcp',
      title: 'BrowserOS MCP server',
      version: deps.version,
    },
    { capabilities: { logging: {} }, instructions: MCP_INSTRUCTIONS },
  )

  server.server.setRequestHandler(SetLevelRequestSchema, () => {
    return {}
  })

  registerTools(server, {
    browserSession: deps.browserSession,
    defaultWindowId: deps.defaultWindowId,
    defaultTabGroupId: deps.defaultTabGroupId,
    executionDir: deps.executionDir,
    remoteAgentHarness: deps.remoteAgentHarness,
  })

  deps.klavis?.registerMcpTools(server, deps.connectorScope)

  return server
}
