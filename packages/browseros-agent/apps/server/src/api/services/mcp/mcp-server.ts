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
import { registerWorkspaceMcpTools } from './register-workspace-mcp'
import type { WorkspaceStore } from '../../../lib/workspace/workspace-store'
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
  /** Workspace persistence scoped to the current MCP conversation. */
  workspaceStore?: WorkspaceStore
  workspaceConversationId?: string
}

/** Creates a per-request BrowserOS MCP server with tools for the requested surface. */
export function createMcpServer(deps: McpServiceDeps) {
  const selectedServerNames = deps.connectorScope?.selectedServerNames ?? []
  const instructions =
    deps.workspaceStore &&
    deps.workspaceConversationId &&
    deps.workspaceConversationId !== 'ephemeral'
      ? `${MCP_INSTRUCTIONS}\n\n## Research Workspace\nCall research_get_session first. Use workspace_create_source for each page, workspace_create_record for verified structured results, and workspace_save_asset for evidence files or screenshots. Update plan steps and call research_update_status only after verifying coverage and source support. Do not store private chain-of-thought.`
      : MCP_INSTRUCTIONS
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
    instructions,
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

  if (deps.workspaceStore && deps.workspaceConversationId) {
    registerWorkspaceMcpTools(server, {
      store: deps.workspaceStore,
      conversationId: deps.workspaceConversationId,
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
