import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { BrowserSession } from '../../../browser/core/session'
import type { BrowserOutputFileAccess } from '../../../tools/browser/output-file'
import {
  type BrowserToolDefaults,
  registerBrowserTools,
} from '../../../tools/browser/register'
import { registerFilesystemMcpTools } from '../../../tools/filesystem/register-mcp'

export interface RemoteAgentHarnessTools {
  outputFileAccess: BrowserOutputFileAccess
}

export interface RegisterToolsDeps extends BrowserToolDefaults {
  browserSession: BrowserSession
  executionDir: string
  remoteAgentHarness?: RemoteAgentHarnessTools
}

/** Registers BrowserOS MCP tools for the current request. */
export function registerTools(
  mcpServer: McpServer,
  deps: RegisterToolsDeps,
): void {
  const defaults = {
    defaultWindowId: deps.defaultWindowId,
    defaultTabGroupId: deps.defaultTabGroupId,
  }

  registerBrowserTools(mcpServer, deps.browserSession, defaults, {
    outputFileAccess: deps.remoteAgentHarness?.outputFileAccess,
  })

  if (deps.remoteAgentHarness) {
    registerFilesystemMcpTools(mcpServer, deps.executionDir, {
      outputFileAccess: deps.remoteAgentHarness.outputFileAccess,
    })
  }
}
