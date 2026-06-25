/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tool-name matcher for "nudge" tools. The UI stream translator drops
 * the upstream tool_call event for these so the renderer doesn't
 * briefly show a generic tool block next to the connect card. The
 * card itself comes from `app_connection_request`, emitted by the
 * /mcp/nudge tool handler via the active-turn registry.
 *
 * Accepts both the bare `suggest_app_connection` and any
 * namespace-prefixed form. acpx-ai-provider stringifies the runtime's
 * tool title (typically `Tool: <server>/<name>`) into the title field;
 * the suffix check tolerates that prefix.
 *
 * Mirrors agent-company's apps/desktop/src/main/chat/nudges.ts so the
 * suppression rule has identical semantics across both products.
 */

export function isNudgeToolName(toolName: string): boolean {
  return (
    toolName === 'suggest_app_connection' ||
    toolName.endsWith('/suggest_app_connection')
  )
}
