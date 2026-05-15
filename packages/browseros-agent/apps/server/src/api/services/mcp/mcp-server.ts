/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { SetLevelRequestSchema } from '@modelcontextprotocol/sdk/types.js'
import type { Browser } from '../../../browser/browser'
import type { ToolRegistry } from '../../../tools/tool-registry'
import { MCP_INSTRUCTIONS } from './mcp-prompt'
import {
  type KlavisProxyHandle,
  registerKlavisTools,
} from './register-klavis-mcp'
import { registerTools } from './register-mcp'
import { N8NProvider, createN8NTools } from '../../../tools/n8n-provider'
import { MultimodalService } from '../../../services/accessibility/multimodal-service'

export interface McpServiceDeps {
  version: string
  registry: ToolRegistry
  browser: Browser
  executionDir: string
  resourcesDir: string
  klavisProxy?: KlavisProxyHandle | null
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
  })

  // Register Klavis proxy tools (if connected)
  if (deps.klavisProxy) {
    registerKlavisTools(server, deps.klavisProxy)
  }

  // Register n8n automation tools
  const n8nProvider = new N8NProvider();
  const n8nTools = createN8NTools(n8nProvider);
  n8nTools.forEach(tool => {
    server.registerTool(
      tool.name,
      {
        description: tool.description,
        inputSchema: tool.inputSchema as any,
      },
      tool.execute as any
    );
  });

  // Register accessibility multimodal tools
  const multimodalService = new MultimodalService();
  multimodalService.getTools().forEach(tool => {
    server.registerTool(
      tool.name,
      {
        description: tool.description,
        inputSchema: tool.inputSchema as any,
      },
      tool.execute as any
    );
  });

  return server
}
