/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Typed wrappers around the singleton McpManager. The API + frontend
 * consume these instead of touching the upstream library directly.
 */

import {
  type AgentId,
  type AgentInfo,
  AgentNotSupportedError,
  detectInstalledAgents,
  ForeignEntryError,
  isAgentSupported,
  type McpHttpSpec,
  type McpServerSpec,
  type McpStdioSpec,
  resolveAgentSurface,
  UnsupportedTransportError,
} from 'agent-mcp-manager'
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
 * Agents the upstream library supports but BrowserOS deliberately
 * does not surface in the Integrations panel. Today: Gemini CLI's
 * MCP HTTP support is not stable enough to one-click-install
 * against. Users who actually want it can still copy-paste the
 * manual setup snippet from the disclosure on the same page.
 */
const HIDDEN_AGENTS: ReadonlySet<string> = new Set(['gemini'])

/**
 * The two server-names BrowserOS manages in the manifest. Iterating
 * both is what `listAgents` + `reconcileUrl` need to do.
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
 * `resolveAgentSurface` so we stay in lock-step with whatever
 * upstream agent-mcp-manager classifies as http-capable. Agents
 * that only accept stdio (claude-desktop, codex, …) get wrapped
 * via `npx mcp-remote <url>` so a stdio client still ends up
 * talking to the local HTTP MCP endpoint.
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
 * Detects every supported agent on disk and reports BrowserOS's link
 * state per agent. Detection is injectable so tests can avoid the
 * real filesystem-walking implementation.
 */
export async function listAgents(
  options: { detect?: DetectInstalledAgentsFn } = {},
): Promise<McpAgentRow[]> {
  const mgr = getMcpManager()
  const detect = options.detect ?? detectInstalledAgents
  const [detectedRaw, links] = await Promise.all([detect(), mgr.listLinks()])
  const detected = detectedRaw.filter((a) => !HIDDEN_AGENTS.has(a.id))
  const linkedSet = new Set(
    links
      .filter((l) => BROWSEROS_SERVER_NAMES.includes(l.serverName))
      .map((l) => l.agent),
  )
  return detected.map((a) => ({
    id: a.id,
    displayName: a.displayName,
    installed: a.installed,
    linked: linkedSet.has(a.id),
    configPath: a.configPath,
  }))
}

/**
 * Install BrowserOS into the given agent's config. Idempotent: a
 * second call against the same agent + URL is a no-op at the disk
 * layer; if the URL drifted, the older entry is replaced before
 * linking. Stdio-only agents are linked under a separate server
 * name so each transport keeps its own manifest entry.
 *
 * Also sweeps the OPPOSITE server name's link for this agent.
 * Without this, an agent that was first installed under the http
 * server `browseros` and later re-routed to stdio by the upstream
 * catalog (or vice versa) would end up double-linked, with the
 * stale entry surviving every uninstall click that targets only
 * the current planFor() server.
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

  // `add` overwrites when the entry already exists; safe to call
  // unconditionally on every install click so a URL drift gets
  // caught even outside the boot-time reconciler.
  await mgr.add({ name: serverName, spec })
  await mgr.link({ serverName, agent: agentId })
  logger.info('Installed BrowserOS MCP into agent', {
    agent: agentId,
    serverName,
  })
  return { success: true }
}

/**
 * Uninstall BrowserOS from the given agent's config. Tries every
 * server name BrowserOS manages because the same agent may be
 * linked under either `browseros` (http) or `browseros-stdio`
 * depending on when it was last installed: the upstream catalog's
 * transport classification for a given agent can flip between
 * library versions, and a stale link under the prior server name
 * would otherwise survive forever.
 *
 * Returns success when at least one server-name unlink completed.
 * Surfaces ForeignEntryError when the user hand-edited the disk
 * entry past what the manifest tracks; that case can only be
 * cleaned up manually.
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
      await mgr.unlink({ serverName, agent: agentId })
      logger.info('Uninstalled BrowserOS MCP from agent', {
        agent: agentId,
        serverName,
      })
    } catch (err) {
      if (err instanceof ForeignEntryError) {
        foreignError = err
        continue
      }
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
 * ForeignEntryError is swallowed (the user hand-edited the foreign
 * entry; let them keep it and overwrite the manifest below). Any
 * other error rethrows so install fails loudly.
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
  if (err instanceof UnsupportedTransportError) {
    return {
      message: `This agent does not support BrowserOS's MCP transport. ${err.message}`,
      status: 400,
    }
  }
  const message = err instanceof Error ? err.message : String(err)
  return { message, status: 500 }
}
