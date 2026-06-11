/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Boot-time URL drift detector. When BrowserOS restarts on a
 * different port (port collision, bun reload, etc.) every agent
 * config that previously linked to BrowserOS still points at the
 * stale URL. The reconciler reads the manifest, compares the
 * recorded URL on each managed server entry against the just-bound
 * URL, and if they differ, replays unlink + link for every
 * previously-linked agent with the new URL.
 *
 * Two manifest entries are managed independently:
 *   - `browseros` — HTTP spec for HTTP-native agents
 *   - `browseros-stdio` — stdio spec wrapping `npx mcp-remote <url>`
 *
 * The reconciler is fire-and-forget at boot. Per-agent failures
 * (e.g. permission denied on someone's config directory) get
 * warn-logged so a single broken agent cannot block the others.
 */

import type {
  InstalledServer,
  McpHttpSpec,
  McpStdioSpec,
} from 'agent-mcp-manager'
import { logger } from '../logger'
import {
  BROWSEROS_MCP_SERVER_NAME,
  BROWSEROS_MCP_STDIO_SERVER_NAME,
  getMcpManager,
} from './manager'
import type { McpAgentId, ReconcileResult } from './types'

export interface ReconcileUrlInput {
  /** The MCP URL the running server bound to, e.g. http://127.0.0.1:9100/mcp */
  currentUrl: string
}

/**
 * Extracts the embedded BrowserOS URL from a managed entry so the
 * reconciler can short-circuit when nothing drifted. Returns null
 * for shapes we don't recognise (e.g. user-edited spec).
 */
function recordedUrl(server: InstalledServer): string | null {
  if (server.spec.transport === 'http') return server.spec.url
  if (server.spec.transport === 'stdio') {
    if (server.spec.command !== 'npx') return null
    const args = server.spec.args ?? []
    const idx = args.indexOf('mcp-remote')
    if (idx < 0) return null
    return args[idx + 1] ?? null
  }
  return null
}

/**
 * Rebuilds a server entry with a fresh URL while preserving its
 * transport flavour. Called per managed name when the URL drifted.
 */
function rebuildSpec(
  serverName: string,
  currentUrl: string,
): McpHttpSpec | McpStdioSpec {
  if (serverName === BROWSEROS_MCP_STDIO_SERVER_NAME) {
    return {
      transport: 'stdio',
      command: 'npx',
      args: ['mcp-remote', currentUrl],
    }
  }
  return { transport: 'http', url: currentUrl }
}

export async function reconcileUrl(
  input: ReconcileUrlInput,
): Promise<ReconcileResult> {
  const mgr = getMcpManager()
  const servers = await mgr.listServers()

  const managedNames = [
    BROWSEROS_MCP_SERVER_NAME,
    BROWSEROS_MCP_STDIO_SERVER_NAME,
  ]
  const affected: McpAgentId[] = []
  let didAnything = false

  for (const name of managedNames) {
    const existing = servers.find((s) => s.name === name)
    if (!existing) continue
    if (recordedUrl(existing) === input.currentUrl) continue

    didAnything = true
    const previouslyLinked = Object.keys(existing.links) as McpAgentId[]

    // Wipe stale entry from the manifest + every linked agent's
    // config, then add the fresh entry and replay the link per agent.
    await mgr.remove({ serverName: name, unlinkFirst: true })
    await mgr.add({ name, spec: rebuildSpec(name, input.currentUrl) })

    for (const agent of previouslyLinked) {
      try {
        await mgr.link({ serverName: name, agent })
        affected.push(agent)
      } catch (err) {
        logger.warn('MCP manager failed to relink agent after URL drift', {
          agent,
          serverName: name,
          error: err instanceof Error ? err.message : String(err),
        })
      }
    }
  }

  if (!didAnything) {
    return { action: 'noop', affectedAgents: [] }
  }

  logger.info('MCP manager reconciled BrowserOS URL', {
    newUrl: input.currentUrl,
    relinked: affected,
  })
  return { action: 'updated', affectedAgents: affected }
}
