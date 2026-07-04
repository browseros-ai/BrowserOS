/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { BrowserSession } from '@browseros/browser-core/core/session'
import { createBrowserMcpServer } from '@browseros/browser-mcp/mcp-server'
import { logger } from '../../../lib/logger'
import { metrics } from '../../../lib/metrics'
import { registerFilesystemMcpTools } from '../../../tools/filesystem/register-mcp'
import { shouldLogToolRegistration } from '../../../tools/registration-log-sampling'
import type { ConnectorToolScope, KlavisService } from '../klavis'
import type { ServerActivity } from '../server-activity'
import { MCP_INSTRUCTIONS } from './mcp-prompt'
import type { RemoteAgentHarnessTools } from './register-mcp'

export interface McpServiceDeps {
  version: string
  browserSession: BrowserSession
  klavis?: KlavisService
  connectorScope?: ConnectorToolScope
  defaultWindowId?: number
  defaultTabGroupId?: string
  executionDir: string
  remoteAgentHarness?: RemoteAgentHarnessTools
  activity?: ServerActivity
}

/** Creates a per-request BrowserOS MCP server with tools for the requested surface. */
export function createMcpServer(deps: McpServiceDeps) {
  const selectedServerNames = deps.connectorScope?.selectedServerNames ?? []
  logger.debug('Creating BrowserOS MCP server', {
    version: deps.version,
    remoteAgentHarness: Boolean(deps.remoteAgentHarness),
    selectedServerNames,
    selectedServerCount: selectedServerNames.length,
    defaultWindowId: deps.defaultWindowId,
    defaultTabGroupId: deps.defaultTabGroupId,
  })

  const server = createBrowserMcpServer({
    name: 'browseros_mcp',
    title: 'BrowserOS MCP server',
    version: deps.version,
    browserSession: deps.browserSession,
    defaultWindowId: deps.defaultWindowId,
    defaultTabGroupId: deps.defaultTabGroupId,
    instructions: MCP_INSTRUCTIONS,
    registration: {
      outputFileAccess: deps.remoteAgentHarness?.outputFileAccess,
      logger,
      onToolExecutionStart: () => deps.activity?.beginMcpToolExecution(),
      onToolExecutionEnd: () => deps.activity?.endMcpToolExecution(),
      onToolExecuted: (event) => metrics.log('tool_executed', event),
      shouldLogToolRegistration,
      source: 'mcp',
    },
  })

  if (deps.remoteAgentHarness) {
    logger.debug('Registering remote harness filesystem MCP tools', {
      executionDir: deps.executionDir,
    })
    registerFilesystemMcpTools(server, deps.executionDir, {
      outputFileAccess: deps.remoteAgentHarness.outputFileAccess,
    })
  }

  deps.klavis?.registerMcpTools(server, deps.connectorScope)
  logger.debug('BrowserOS MCP server created', {
    remoteAgentHarness: Boolean(deps.remoteAgentHarness),
    selectedServerNames,
    selectedServerCount: selectedServerNames.length,
  })

  return server
}
