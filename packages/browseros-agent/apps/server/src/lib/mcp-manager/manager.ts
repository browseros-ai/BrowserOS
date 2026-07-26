/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Singleton accessor for the `@browseros/agent-mcp-manager` bound API.
 * The workspaceDir is pinned to `<getBrowserosDir()>/mcp-manager` so
 * the manifest of which agents BrowserOS has installed itself into
 * lives next to the rest of the BrowserOS state and travels with the
 * install.
 *
 * Since 0.0.4 the library exposes a functional surface. `bind()`
 * pre-fills the workspaceDir on every verb so call sites stay
 * `mgr.link({...})`, `mgr.list()`, etc. Scope defaults to 'system'
 * on each verb; per-call `scope` overrides are still available via
 * the input object.
 */

import { join } from 'node:path'
import { type BoundApi, bind } from '@browseros/agent-mcp-manager'
import { getBrowserosDir } from '../browseros-dir'

/**
 * Server-name BrowserOS registers itself under for agents that speak
 * MCP over HTTP natively (Claude Code, Codex, Cursor, OpenCode,
 * Antigravity, VS Code, Zed). Stdio-only agents, when supported, get
 * a separate entry under `BROWSEROS_MCP_STDIO_SERVER_NAME` below.
 */
export const BROWSEROS_MCP_SERVER_NAME = 'browseros'

/**
 * Server-name BrowserOS registers itself under for stdio-only agents.
 * The spec wraps `npx mcp-remote <url>` so a stdio client can speak
 * to the BrowserOS HTTP MCP endpoint. Kept as a separate manifest
 * entry from the HTTP one so each carries its own spec and can be
 * reconciled independently. Every surfaced agent currently supports
 * HTTP, so this entry only exists to sweep legacy stdio links left by
 * earlier installs.
 */
export const BROWSEROS_MCP_STDIO_SERVER_NAME = 'browseros-stdio'

let cached: BoundApi | null = null

export function getMcpManagerWorkspaceDir(): string {
  return join(getBrowserosDir(), 'mcp-manager')
}

/** Singleton accessor, lazily constructs on first call. */
export function getMcpManager(): BoundApi {
  if (!cached) cached = bind(getMcpManagerWorkspaceDir())
  return cached
}

/** Reset the cached instance. Tests only. */
export function resetMcpManagerForTesting(): void {
  cached = null
}

/** Test seam: inject a stub manager so unit tests can avoid touching disk. */
export function setMcpManagerForTesting(stub: BoundApi): void {
  cached = stub
}
