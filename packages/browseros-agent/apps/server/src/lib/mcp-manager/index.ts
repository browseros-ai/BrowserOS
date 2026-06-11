/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export {
  BROWSEROS_MCP_SERVER_NAME,
  BROWSEROS_MCP_STDIO_SERVER_NAME,
  getMcpManager,
  resetMcpManagerForTesting,
  setMcpManagerForTesting,
} from './manager'
export { type ReconcileUrlInput, reconcileUrl } from './reconcile'
export {
  humaniseInstallError,
  installInto,
  listAgents,
  uninstallFrom,
} from './service'
export type {
  InstallAgentResult,
  McpAgentId,
  McpAgentRow,
  ReconcileResult,
  UninstallAgentResult,
} from './types'
