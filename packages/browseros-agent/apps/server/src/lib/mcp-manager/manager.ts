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

/**
 * Server-name BrowserOS registers itself under for agents that speak
 * MCP over HTTP natively (Claude Code, Cursor, VS Code, Codex, Zed).
 * Stdio-only agents — when supported — get a separate entry under
 * `BROWSEROS_MCP_STDIO_SERVER_NAME` below.
 */
export const BROWSEROS_MCP_SERVER_NAME = 'browseros'

/**
 * Server-name BrowserOS registers itself under for stdio-only agents.
 * The spec wraps `npx mcp-remote <url>` so a stdio client can speak
 * to the BrowserOS HTTP MCP endpoint. Kept as a separate manifest
 * entry from the HTTP one so each carries its own spec and can be
 * reconciled independently. No surfaced agent currently uses this
 * branch — Claude Desktop is hidden from the Integrations panel
 * because its stdio bridge requires Node on the user's machine.
 */
export const BROWSEROS_MCP_STDIO_SERVER_NAME = 'browseros-stdio'

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
