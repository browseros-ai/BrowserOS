/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Choke-point relink for a managed MCP server. Every claw-server
 * write path (Connect button, profile install, profile reconcile,
 * boot URL migration) funnels through here so the transport rule
 * lives in one place. On failure, restores the previous link so a
 * partial write does not orphan the entry.
 *
 * Since agent-mcp-manager 0.0.4 the library emits the Claude Code
 * `type: "http"` transport tag natively at the catalog layer, so
 * the post-write fixup that used to live here retired.
 */

import type {
  AgentId,
  BoundApi,
  LinkPlanSummary,
  McpServerSpec,
} from 'agent-mcp-manager'

interface RelinkManagedServerOptions {
  mgr: BoundApi
  serverName: string
  agent: AgentId
  spec: McpServerSpec
  allowOverwrite?: boolean
}

/** Rewrites a managed MCP link for URL drift and restores the old link if replacement fails. */
export async function relinkManagedServer({
  mgr,
  serverName,
  agent,
  spec,
  allowOverwrite,
}: RelinkManagedServerOptions): Promise<LinkPlanSummary> {
  const previousSpec = await findExistingSpec(mgr, serverName)
  try {
    return await mgr.link({
      server: { name: serverName, spec },
      agent,
      ...(allowOverwrite ? { allowOverwrite } : {}),
    })
  } catch (err) {
    if (previousSpec) {
      try {
        await mgr.link({
          server: { name: serverName, spec: previousSpec },
          agent,
          ...(allowOverwrite ? { allowOverwrite } : {}),
        })
      } catch (restoreErr) {
        const relinkMessage = err instanceof Error ? err.message : String(err)
        const restoreMessage =
          restoreErr instanceof Error ? restoreErr.message : String(restoreErr)
        throw new Error(
          `Could not relink ${serverName}: ${relinkMessage}; also failed to restore previous link: ${restoreMessage}`,
        )
      }
    }
    throw err
  }
}

async function findExistingSpec(
  mgr: BoundApi,
  serverName: string,
): Promise<McpServerSpec | null> {
  const servers = await mgr.list()
  return servers.find((server) => server.name === serverName)?.spec ?? null
}
