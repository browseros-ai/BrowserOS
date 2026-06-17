/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Singleton accessor for `agent-mcp-manager`. Manifest lives at
 * `<browserosDir>/mcp-interface/mcp-manager` so the per-cockpit-
 * agent server entries stay isolated from the BrowserOS-wide entry
 * `apps/server` manages under `<browserosDir>/mcp-manager`.
 *
 * The library writes config to the user's per-harness MCP file
 * (e.g. Claude Desktop's `~/Library/Application Support/Claude/
 * claude_desktop_config.json`, Cursor's `~/.cursor/mcp.json`).
 * Scope is always 'system' here since cockpit agents are user-wide.
 */

import { join } from 'node:path'
import { createMcpManager, type McpManager } from 'agent-mcp-manager'
import { getInterfaceDir } from './browseros-dir'

let cached: McpManager | null = null

export function getMcpManager(): McpManager {
  if (!cached) {
    cached = createMcpManager({
      workspaceDir: join(getInterfaceDir(), 'mcp-manager'),
      scope: 'system',
    })
  }
  return cached
}

/** Test seam: drop the cached manager so the next caller rebuilds it. */
export function resetMcpManagerForTesting(): void {
  cached = null
}

/** Test seam: inject a stub manager. */
export function setMcpManagerForTesting(stub: McpManager): void {
  cached = stub
}
