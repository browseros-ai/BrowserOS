/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  AgentId,
  LinkServerResult,
  McpManager,
  McpServerLink,
  McpServerSpec,
} from 'agent-mcp-manager'

interface RelinkManagedServerOptions {
  mgr: McpManager
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
}: RelinkManagedServerOptions): Promise<LinkServerResult> {
  const existingLink = await findExistingLink(mgr, serverName, agent)
  const previousSpec = existingLink
    ? await findExistingSpec(mgr, serverName)
    : null

  await mgr.add({ name: serverName, spec })
  try {
    if (existingLink) {
      await mgr.unlink({
        serverName,
        agent,
        configPath: existingLink.configPath,
      })
    }
    return await mgr.link({
      serverName,
      agent,
      ...(existingLink ? { configPath: existingLink.configPath } : {}),
      ...(allowOverwrite ? { allowOverwrite } : {}),
    })
  } catch (err) {
    if (existingLink && previousSpec) {
      try {
        await mgr.add({ name: serverName, spec: previousSpec })
        await mgr.link({
          serverName,
          agent,
          configPath: existingLink.configPath,
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

async function findExistingLink(
  mgr: McpManager,
  serverName: string,
  agent: AgentId,
): Promise<McpServerLink | null> {
  const links = await mgr.listLinks({ serverNames: [serverName] })
  return links.find((link) => link.agent === agent) ?? null
}

async function findExistingSpec(
  mgr: McpManager,
  serverName: string,
): Promise<McpServerSpec | null> {
  const servers = await mgr.listServers()
  return servers.find((server) => server.name === serverName)?.spec ?? null
}
