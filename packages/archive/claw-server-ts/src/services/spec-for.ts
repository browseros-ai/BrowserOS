/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Picks the right `McpServerSpec` shape for a given harness agent id.
 * Transport capability is sourced from the agent-mcp-manager catalog
 * (`resolveAgentSurface`) so a bump like Codex moving from stdio-only
 * to http-capable (0.0.3) is picked up automatically without editing
 * a parallel allow-list here. Stdio-only agents fall back to wrapping
 * the URL via `npx mcp-remote`, the same approach `apps/server` uses.
 *
 * Used by the shared install in `browseros-connect.ts` so the
 * transport rule stays separate from connection orchestration.
 */

import {
  type AgentId,
  type McpServerSpec,
  resolveAgentSurface,
} from '@browseros/agent-mcp-manager'

export function specFor(agentId: AgentId, mcpUrl: string): McpServerSpec {
  const surface = resolveAgentSurface(agentId, 'system')
  if (surface.supportedTransports.includes('http')) {
    return { transport: 'http', url: mcpUrl }
  }
  return {
    transport: 'stdio',
    command: 'npx',
    args: ['mcp-remote', mcpUrl],
  }
}
