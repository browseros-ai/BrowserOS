/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Boot-time URL drift detector. When BrowserOS restarts on a
 * different port (port collision, bun reload, etc.) every agent
 * config that previously linked to BrowserOS still points at the
 * stale URL. The reconciler reads the manifest, compares the
 * recorded `browseros` URL against the just-bound URL, and if they
 * differ, replays unlink + link for every previously-linked agent
 * with the new URL.
 *
 * The reconciler is fire-and-forget at boot. Per-agent failures
 * (e.g. permission denied on someone's config directory) get
 * warn-logged so a single broken agent cannot block the others.
 */

import type { McpHttpSpec } from 'agent-mcp-manager'
import { logger } from '../logger'
import { BROWSEROS_MCP_SERVER_NAME, getMcpManager } from './manager'
import type { McpAgentId, ReconcileResult } from './types'

export interface ReconcileUrlInput {
  /** The MCP URL the running server bound to, e.g. http://127.0.0.1:9100/mcp */
  currentUrl: string
}

export async function reconcileUrl(
  input: ReconcileUrlInput,
): Promise<ReconcileResult> {
  const mgr = getMcpManager()

  const servers = await mgr.listServers()
  const existing = servers.find((s) => s.name === BROWSEROS_MCP_SERVER_NAME)
  if (!existing) {
    return { action: 'noop', affectedAgents: [] }
  }
  if (
    existing.spec.transport === 'http' &&
    existing.spec.url === input.currentUrl
  ) {
    return { action: 'noop', affectedAgents: [] }
  }

  const previouslyLinked = Object.keys(existing.links) as McpAgentId[]

  // Wipe stale entry from the manifest + every linked agent's config,
  // then add the fresh entry and replay the link per agent.
  await mgr.remove({ serverName: BROWSEROS_MCP_SERVER_NAME, unlinkFirst: true })
  const desiredSpec: McpHttpSpec = { transport: 'http', url: input.currentUrl }
  await mgr.add({ name: BROWSEROS_MCP_SERVER_NAME, spec: desiredSpec })

  const relinked: McpAgentId[] = []
  for (const agent of previouslyLinked) {
    try {
      await mgr.link({ serverName: BROWSEROS_MCP_SERVER_NAME, agent })
      relinked.push(agent)
    } catch (err) {
      logger.warn('MCP manager failed to relink agent after URL drift', {
        agent,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  logger.info('MCP manager reconciled BrowserOS URL', {
    newUrl: input.currentUrl,
    relinked,
  })
  return { action: 'updated', affectedAgents: relinked }
}
