/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Picks the right `McpServerSpec` shape for a given harness agent id.
 * Most harnesses speak HTTP MCP natively, so a `{ transport: 'http',
 * url }` entry is enough. Codex is stdio-only, so we wrap the URL via
 * `npx mcp-remote` the same way `apps/server` does.
 *
 * Used by both the legacy per-agent install (`harness-install.ts`)
 * and the v2 single-endpoint install (`browseros-connect.ts`) so the
 * stdio-wrapping rule lives in one place.
 */

import type { AgentId, McpServerSpec } from 'agent-mcp-manager'

const STDIO_ONLY: ReadonlySet<AgentId> = new Set<AgentId>(['codex'])

export function specFor(agentId: AgentId, mcpUrl: string): McpServerSpec {
  if (STDIO_ONLY.has(agentId)) {
    return {
      transport: 'stdio',
      command: 'npx',
      args: ['mcp-remote', mcpUrl],
    }
  }
  return { transport: 'http', url: mcpUrl }
}
