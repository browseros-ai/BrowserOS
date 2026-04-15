/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AclRule } from '@browseros/shared/types/acl'
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { SetLevelRequestSchema } from '@modelcontextprotocol/sdk/types.js'
import type { Browser } from '../../../browser/browser'
import type { ToolRegistry } from '../../../tools/tool-registry'
import {
  type KlavisProxyRef,
  registerKlavisTools,
} from '../klavis/strata-proxy'
import { MCP_INSTRUCTIONS } from './mcp-prompt'
import { registerTools } from './register-mcp'

export interface McpServiceDeps {
  version: string
  registry: ToolRegistry
  browser: Browser
  executionDir: string
  resourcesDir: string
  aclRules?: AclRule[]
  klavisRef?: KlavisProxyRef
}

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

  // Register browser tools
  registerTools(server, deps.registry, {
    browser: deps.browser,
    directories: {
      workingDir: deps.executionDir,
      resourcesDir: deps.resourcesDir,
    },
    aclRules: deps.aclRules,
  })

  // Register Klavis proxy tools (if connected via background init)
  if (deps.klavisRef?.handle) {
    registerKlavisTools(server, deps.klavisRef.handle)
  }

  return server
}
