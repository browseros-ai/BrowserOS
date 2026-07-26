/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Typed wrappers around the singleton bound MCP manager. The API +
 * frontend consume these instead of touching the upstream library
 * directly.
 *
 * Since 0.0.4 the library is functional: `link()` upserts the
 * manifest entry and writes the agent config in one call (no separate
 * `add()`), and `disconnect()` folds unlink + drop-if-last together.
 * The Claude Code HTTP `type` field is now emitted by the catalog
 * layer, so no post-write transport-tag fixup is needed here.
 */

import {
  type AgentId,
  type AgentInfo,
  AgentNotInstalledError,
  AgentNotSupportedError,
  detectInstalledAgents,
  ForeignEntryError,
  isAgentSupported,
  type McpHttpSpec,
  type McpServerSpec,
  type McpStdioSpec,
  resolveAgentSurface,
  ServerNotFoundError,
  UnsupportedTransportError,
} from '@browseros/agent-mcp-manager'
import { logger } from '../logger'
import {
  BROWSEROS_MCP_SERVER_NAME,
  BROWSEROS_MCP_STDIO_SERVER_NAME,
  getMcpManager,
} from './manager'
import type {
  InstallAgentResult,
  McpAgentRow,
  UninstallAgentResult,
} from './types'

export type DetectInstalledAgentsFn = () => Promise<AgentInfo[]>

/**
 * The curated harness set BrowserOS surfaces in the Integrations
 * panel, mirroring the cockpit app (claw-server). Order is the render
 * order in the UI. Only these agents appear, and only when detected
 * on the machine or already linked.
 *
 * The upstream catalog knows ~two dozen clients; the ones left out
 * here are deliberately not one-click surfaces:
 *
 * - `gemini`: HTTP MCP support is not stable enough to one-click
 *   install against.
 * - `claude-desktop`: Anthropic's `claude_desktop_config.json` parser
 *   only validates stdio entries, and the recommended `npx mcp-remote`
 *   bridge needs Node on the user's machine.
 *
 * Both remain reachable via the manual setup snippet on the same page.
 */
export const CURATED_AGENTS: readonly AgentId[] = [
  'claude-code',
  'codex',
  'cursor',
  'opencode',
  'antigravity',
  'vscode',
  'zed',
]

/**
 * The two server-names BrowserOS manages in the manifest. Every
 * surfaced agent supports HTTP, so installs only ever write the HTTP
 * entry; the stdio name is swept on uninstall to clean up any legacy
 * link left by an earlier catalog version.
 */
const BROWSEROS_SERVER_NAMES: readonly string[] = [
  BROWSEROS_MCP_SERVER_NAME,
  BROWSEROS_MCP_STDIO_SERVER_NAME,
]

interface AgentServerPlan {
  serverName: string
  spec: McpServerSpec
}

/**
 * Pick the server name + spec a given agent should be linked under.
 *
 * Transport routing is sourced from the library's catalog via
 * `resolveAgentSurface` so we stay in lock-step with whatever upstream
 * agent-mcp-manager classifies as http-capable. Agents that only
 * accept stdio get wrapped via `npx mcp-remote <url>` so a stdio
 * client still ends up talking to the local HTTP MCP endpoint.
 */
function planFor(agentId: AgentId, currentUrl: string): AgentServerPlan {
  const surface = resolveAgentSurface(agentId, 'system')
  const supportsHttp = surface.supportedTransports.includes('http')
  if (!supportsHttp) {
    const spec: McpStdioSpec = {
      transport: 'stdio',
      command: 'npx',
      args: ['mcp-remote', currentUrl],
    }
    return { serverName: BROWSEROS_MCP_STDIO_SERVER_NAME, spec }
  }
  const spec: McpHttpSpec = { transport: 'http', url: currentUrl }
  return { serverName: BROWSEROS_MCP_SERVER_NAME, spec }
}

/**
 * Detects the curated agents on disk and reports BrowserOS's link
 * state per agent. Only agents installed on the machine or already
 * linked are returned, in curated order. Detection is injectable so
 * tests can avoid the real filesystem-walking implementation.
 */
export async function listAgents(
  options: { detect?: DetectInstalledAgentsFn } = {},
): Promise<McpAgentRow[]> {
  const mgr = getMcpManager()
  const detect = options.detect ?? detectInstalledAgents
  const [detectedRaw, links] = await Promise.all([
    detect(),
    mgr.listLinks({ serverNames: [...BROWSEROS_SERVER_NAMES] }),
  ])
  const linkedByAgent = new Map(links.map((l) => [l.agent, l]))
  const detectedById = new Map(detectedRaw.map((a) => [a.id, a]))

  const rows: McpAgentRow[] = []
  for (const id of CURATED_AGENTS) {
    const detected = detectedById.get(id)
    const link = linkedByAgent.get(id)
    const linked = link !== undefined
    // An existing BrowserOS link means a working install regardless of
    // what disk detection reports, so linked wins the `installed` flag.
    const installed = linked || (detected?.installed ?? false)
    // Only surface agents the user actually has, matching the cockpit
    // app: neither installed nor linked means no row.
    if (!installed) continue
    rows.push({
      id,
      displayName: detected?.displayName ?? id,
      installed,
      linked,
      configPath: link?.configPath ?? detected?.configPath ?? null,
    })
  }
  return rows
}

/**
 * Install BrowserOS into the given agent's config. Idempotent: a
 * second call against the same agent + URL is a no-op at the disk
 * layer; if the URL drifted, `link` upserts the entry before writing.
 * Stdio-only agents are linked under a separate server name so each
 * transport keeps its own manifest entry.
 *
 * Also sweeps the OPPOSITE server name's link for this agent. Without
 * this, an agent that was first installed under the http server
 * `browseros` and later re-routed to stdio by the upstream catalog
 * (or vice versa) would end up double-linked, with the stale entry
 * surviving every uninstall click that targets only the current
 * planFor() server.
 */
export async function installInto(
  agentId: string,
  currentUrl: string,
): Promise<InstallAgentResult> {
  if (!isAgentSupported(agentId)) {
    throw new AgentNotSupportedError(agentId)
  }
  const mgr = getMcpManager()
  const { serverName, spec } = planFor(agentId, currentUrl)

  await sweepLegacyLinks(agentId, serverName)

  // `link` upserts the manifest spec and writes the agent config in a
  // single call; `allowOverwrite` replaces a matching foreign entry so
  // a URL drift is caught even outside the boot-time reconciler.
  await mgr.link({
    server: { name: serverName, spec },
    agent: agentId,
    allowOverwrite: true,
  })
  logger.info('Installed BrowserOS MCP into agent', {
    agent: agentId,
    serverName,
  })
  return { success: true }
}

/**
 * Uninstall BrowserOS from the given agent's config. Tries every
 * server name BrowserOS manages because the same agent may be linked
 * under either `browseros` (http) or `browseros-stdio` depending on
 * when it was last installed: the upstream catalog's transport
 * classification for a given agent can flip between library versions,
 * and a stale link under the prior server name would otherwise survive
 * forever.
 *
 * Returns success when no foreign-entry conflict blocked a removal.
 * A missing manifest entry for a server name is a no-op, not an error.
 */
export async function uninstallFrom(
  agentId: string,
): Promise<UninstallAgentResult> {
  if (!isAgentSupported(agentId)) {
    throw new AgentNotSupportedError(agentId)
  }
  const mgr = getMcpManager()
  let foreignError: ForeignEntryError | null = null
  for (const serverName of BROWSEROS_SERVER_NAMES) {
    try {
      await mgr.disconnect({ serverName, agent: agentId, removeIfLast: true })
      logger.info('Uninstalled BrowserOS MCP from agent', {
        agent: agentId,
        serverName,
      })
    } catch (err) {
      if (err instanceof ForeignEntryError) {
        foreignError = err
        continue
      }
      // The agent was never linked under this server name (or the
      // whole entry is gone); nothing to remove, keep sweeping.
      if (err instanceof ServerNotFoundError) continue
      throw err
    }
  }
  if (foreignError) {
    return {
      success: false,
      message:
        'Cannot remove a user-edited entry. Please remove BrowserOS from this agent manually and try again.',
    }
  }
  return { success: true }
}

/**
 * Cleans any pre-existing BrowserOS link for `agentId` under server
 * names other than the one we're about to link under. Best-effort:
 * ForeignEntryError and ServerNotFoundError are swallowed (nothing we
 * own to remove); any other error rethrows so install fails loudly.
 */
async function sweepLegacyLinks(
  agentId: AgentId,
  targetServerName: string,
): Promise<void> {
  const mgr = getMcpManager()
  for (const serverName of BROWSEROS_SERVER_NAMES) {
    if (serverName === targetServerName) continue
    try {
      await mgr.unlink({ serverName, agent: agentId })
    } catch (err) {
      if (err instanceof ForeignEntryError) continue
      if (err instanceof ServerNotFoundError) continue
      throw err
    }
  }
}

export function humaniseInstallError(err: unknown): {
  message: string
  status: number
} {
  if (err instanceof AgentNotSupportedError) {
    return { message: `Agent "${err.agent}" is not supported.`, status: 404 }
  }
  if (err instanceof ForeignEntryError) {
    return {
      message:
        "Cannot replace a user-edited entry. Please remove BrowserOS from this agent's config manually and try again.",
      status: 409,
    }
  }
  if (err instanceof AgentNotInstalledError) {
    return {
      message:
        'This agent is installed but its MCP config location is not writable yet. Open the app once so it creates its config, then try again.',
      status: 409,
    }
  }
  if (err instanceof UnsupportedTransportError) {
    return {
      message: `This agent does not support BrowserOS's MCP transport. ${err.message}`,
      status: 400,
    }
  }
  const message = err instanceof Error ? err.message : String(err)
  return { message, status: 500 }
}

/**
 * Startup self-heal. Disconnects BrowserOS from any agent that is no
 * longer part of the curated surface, e.g. a Gemini or Claude Desktop
 * link created by an earlier build before those were dropped. Leaving
 * them linked would strand a BrowserOS entry in a config the panel can
 * no longer show a disconnect row for. Runs non-blocking at boot;
 * per-agent failures are swallowed so one bad config cannot block the
 * rest. Returns the agents that were actually unlinked.
 */
export async function cleanupNonCuratedLinks(): Promise<AgentId[]> {
  const mgr = getMcpManager()
  const curated = new Set<string>(CURATED_AGENTS)
  let links: Awaited<ReturnType<typeof mgr.listLinks>>
  try {
    links = await mgr.listLinks({ serverNames: [...BROWSEROS_SERVER_NAMES] })
  } catch (err) {
    logger.warn('MCP manager: could not list links for non-curated cleanup', {
      error: err instanceof Error ? err.message : String(err),
    })
    return []
  }
  const stale = [
    ...new Set(links.map((l) => l.agent).filter((a) => !curated.has(a))),
  ]
  const removed: AgentId[] = []
  for (const agent of stale) {
    let didRemove = false
    for (const serverName of BROWSEROS_SERVER_NAMES) {
      try {
        const summary = await mgr.disconnect({
          serverName,
          agent,
          removeIfLast: true,
        })
        if (summary.unlinked) didRemove = true
      } catch (err) {
        if (err instanceof ForeignEntryError) continue
        if (err instanceof ServerNotFoundError) continue
        logger.warn('MCP manager: failed to clean up non-curated link', {
          agent,
          serverName,
          error: err instanceof Error ? err.message : String(err),
        })
      }
    }
    if (didRemove) removed.push(agent)
  }
  if (removed.length > 0) {
    logger.info('MCP manager cleaned up non-curated agent links', {
      agents: removed,
    })
  }
  return removed
}
