/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Singleton wrapper around `createMcpManager`. The workspaceDir is
 * pinned to `<getBrowserosDir()>/mcp-manager` so the manifest of
 * which agents BrowserOS has installed itself into lives next to
 * the rest of the BrowserOS state and travels with the install.
 */

import { join } from 'node:path'
import { createMcpManager, type McpManager } from 'agent-mcp-manager'
import { getBrowserosDir } from '../browseros-dir'

/** Server-name BrowserOS registers itself under in every agent's config. */
export const BROWSEROS_MCP_SERVER_NAME = 'browseros'

let cached: McpManager | null = null

/** Singleton accessor — lazily constructs on first call. */
export function getMcpManager(): McpManager {
  if (cached) return cached
  cached = createMcpManager({
    workspaceDir: join(getBrowserosDir(), 'mcp-manager'),
    scope: 'system',
  })
  return cached
}

/** Reset the cached instance. Tests only. */
export function resetMcpManagerForTesting(): void {
  cached = null
}

/** Test seam: inject a stub manager so unit tests can avoid touching disk. */
export function setMcpManagerForTesting(stub: McpManager): void {
  cached = stub
}
