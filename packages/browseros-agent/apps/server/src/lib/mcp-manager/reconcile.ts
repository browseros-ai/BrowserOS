/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Boot-time self-heal for the BrowserOS MCP integration.
 *
 * Two managed manifest entries carry BrowserOS's server spec:
 *   - `browseros` (HTTP spec for HTTP-native agents)
 *   - `browseros-stdio` (stdio spec wrapping `npx mcp-remote <url>`)
 *
 * When BrowserOS restarts on a different port every agent config that
 * previously linked to BrowserOS still points at the stale URL. The
 * reconciler repairs them. It cannot trust the manifest's own recorded
 * URL as a drift signal: the manifest carries ONE spec per server that
 * every linked agent shares, so if one agent's config write failed
 * during a prior port change, the manifest can read the new URL while
 * that agent's config is still stale. Reading the manifest would then
 * mask the broken agent forever.
 *
 * Instead it asks the library to `rescan()`, which reads every linked
 * agent's config off disk and reports which still carry a BrowserOS
 * entry (`verified`) versus which the user removed by hand (`drifted`
 * / `missing`). Every verified curated entry is re-linked to the
 * current URL, so a stale entry is always repaired regardless of what
 * the shared manifest spec says. Entries the user deleted are left
 * alone so we never resurrect something they removed on purpose.
 *
 * Re-linking is idempotent, so an entry already on the right URL is
 * rewritten with identical content. `link()` also emits the Claude
 * Code HTTP `type` field at the catalog layer, so no post-write fixup
 * runs here. Per-agent failures warn-log and are retried on the next
 * boot; a single broken config cannot block the others.
 */

import type { McpHttpSpec, McpStdioSpec } from '@browseros/agent-mcp-manager'
import { logger } from '../logger'
import {
  BROWSEROS_MCP_SERVER_NAME,
  BROWSEROS_MCP_STDIO_SERVER_NAME,
  getMcpManager,
} from './manager'
import { CURATED_AGENTS, cleanupNonCuratedLinks } from './service'
import type { McpAgentId, ReconcileResult } from './types'

export interface ReconcileUrlInput {
  /** The client-facing MCP URL, e.g. http://127.0.0.1:9100/mcp */
  currentUrl: string
}

const MANAGED_SERVER_NAMES: ReadonlySet<string> = new Set([
  BROWSEROS_MCP_SERVER_NAME,
  BROWSEROS_MCP_STDIO_SERVER_NAME,
])

/**
 * Rebuilds a server spec for the current URL while preserving its
 * transport flavour. The stdio entry wraps `npx mcp-remote <url>`;
 * everything else is native HTTP.
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

/**
 * Repairs every still-present BrowserOS entry to point at `currentUrl`.
 * Uses `rescan()` so a stale entry is caught even when the shared
 * manifest spec already reads the new URL, and so user-removed entries
 * are never resurrected.
 */
export async function reconcileUrl(
  input: ReconcileUrlInput,
): Promise<ReconcileResult> {
  const mgr = getMcpManager()
  const curated = new Set<string>(CURATED_AGENTS)

  let verified: Awaited<ReturnType<typeof mgr.rescan>>['verified']
  try {
    ;({ verified } = await mgr.rescan())
  } catch (err) {
    logger.warn('MCP manager rescan failed; skipping URL reconcile', {
      error: err instanceof Error ? err.message : String(err),
    })
    return { action: 'noop', affectedAgents: [] }
  }

  const targets = verified.filter(
    (v) => MANAGED_SERVER_NAMES.has(v.serverName) && curated.has(v.agent),
  )

  const affected: McpAgentId[] = []
  for (const { serverName, agent } of targets) {
    const spec = rebuildSpec(serverName, input.currentUrl)
    try {
      await mgr.link({
        server: { name: serverName, spec },
        agent,
        allowOverwrite: true,
      })
      affected.push(agent)
    } catch (err) {
      logger.warn('MCP manager failed to relink agent during URL reconcile', {
        agent,
        serverName,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  if (affected.length === 0) {
    return { action: 'noop', affectedAgents: [] }
  }

  logger.info('MCP manager reconciled BrowserOS URL', {
    newUrl: input.currentUrl,
    relinked: affected,
  })
  return { action: 'updated', affectedAgents: affected }
}

export interface SelfHealInput {
  /**
   * The client-facing MCP URL. When omitted (the launching process did
   * not pass one) the URL reconcile is skipped, but the non-curated
   * cleanup still runs since it needs no URL.
   */
  currentUrl?: string
}

/**
 * One-shot boot self-heal, run non-blocking. First disconnects
 * BrowserOS from any agent no longer in the curated surface (Gemini,
 * Claude Desktop from older builds), then repairs the URL on every
 * remaining agent. The two steps are serialised so their manifest
 * writes cannot race.
 */
export async function selfHealMcpLinks(
  input: SelfHealInput,
): Promise<ReconcileResult> {
  await cleanupNonCuratedLinks()
  if (!input.currentUrl) return { action: 'noop', affectedAgents: [] }
  return reconcileUrl({ currentUrl: input.currentUrl })
}
