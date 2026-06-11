/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Public types the BrowserOS mcp-manager service exposes. Mirrors
 * just enough of `agent-mcp-manager`'s surface for the API + UI to
 * consume without leaking the upstream's library types everywhere.
 */

import type { AgentId } from 'agent-mcp-manager'

export type McpAgentId = AgentId

/** Single row in the `/mcp-manager/agents` response. */
export interface McpAgentRow {
  id: McpAgentId
  displayName: string
  /** True when one of the agent's install-check paths resolves on disk. */
  installed: boolean
  /** True when BrowserOS is currently linked into this agent's config. */
  linked: boolean
  /** Absolute path of the agent's config file (when resolvable). */
  configPath: string | null
}

export interface InstallAgentResult {
  success: boolean
  message?: string
}

export interface UninstallAgentResult {
  success: boolean
  message?: string
}

export interface ReconcileResult {
  action: 'noop' | 'updated'
  affectedAgents: McpAgentId[]
}
