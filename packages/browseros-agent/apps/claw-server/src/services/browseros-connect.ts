/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 single-endpoint install layer. Writes one canonical
 * `"BrowserClaw"` entry into the harness's MCP config file via
 * `agent-mcp-manager`, pointing at the slugless canonical URL
 * (`http://127.0.0.1:9200/mcp`). One row per supported harness,
 * idempotent connect / disconnect, list reads through the library's
 * manifest so the UI reflects the current install state within the
 * polling interval.
 */

import type { AgentId } from 'agent-mcp-manager'
import {
  AgentNotInstalledError,
  ForeignEntryError,
  resolveAgentMcpConfigPath,
} from 'agent-mcp-manager'
import { logger } from '../lib/logger'
import { getMcpManager } from '../lib/mcp-manager'
import { tildifyHomePath } from '../lib/tildify'
import { type Harness, harnessEnum } from '../routes/agents/schemas'
import { BROWSEROS_MCP_SERVER_NAME, publicMcpUrl } from '../shared/mcp-url'
import { HARNESS_TO_AGENT_ID } from './harness-install'
import { relinkManagedServer } from './mcp-relink'
import { specFor } from './spec-for'

export interface ConnectionState {
  harness: Harness
  /** True when the harness has a "BrowserClaw" entry pointing at the canonical URL. */
  installed: boolean
  /** Filled when `installed` is true and a real config file was touched. */
  configPath?: string
  /** Stable agent-mcp-manager id for the harness. */
  agentId: AgentId
  /** Single-line human-readable message; surfaced to the UI verbatim. */
  message: string
}

const ALL_HARNESSES: readonly Harness[] = harnessEnum.options

export async function connectBrowserosToHarness(
  harness: Harness,
): Promise<ConnectionState> {
  const agentId = HARNESS_TO_AGENT_ID[harness]
  const mgr = getMcpManager()
  const url = publicMcpUrl()
  try {
    await relinkManagedServer({
      mgr,
      serverName: BROWSEROS_MCP_SERVER_NAME,
      agent: agentId,
      spec: specFor(agentId, url),
      // Take ownership of any prior on-disk BrowserClaw entry that
      // the manifest does not know about. Without this, agent-mcp-
      // manager throws ForeignEntryError to protect the user from
      // clobbering a foreign entry; that safety net is unnecessary
      // for our case because BrowserClaw is the app's own name and
      // any prior entry under it was almost certainly written by
      // an earlier BrowserClaw install (relocated workspace, dev
      // rebuild, or a prior version of the manifest).
      allowOverwrite: true,
    })
    const rawConfigPath = await resolveAgentMcpConfigPath(
      agentId,
      'system',
    ).catch(() => undefined)
    logger.info('connected browseros to harness', {
      harness,
      agent: agentId,
      configPath: rawConfigPath,
    })
    return {
      harness,
      installed: true,
      agentId,
      configPath: tildifyHomePath(rawConfigPath),
      message: `BrowserOS registered as an MCP server in ${harness}.`,
    }
  } catch (err) {
    return failure(harness, agentId, err, 'connect')
  }
}

export async function disconnectBrowserosFromHarness(
  harness: Harness,
): Promise<ConnectionState> {
  const agentId = HARNESS_TO_AGENT_ID[harness]
  const mgr = getMcpManager()
  try {
    // `disconnect` is the 0.0.4 atomic primitive: unlinks the agent
    // AND drops the manifest entry only if no other agents remain
    // linked. Replaces the pre-0.0.4 three-step unlink + listLinks +
    // conditional remove dance which had a race window where a
    // concurrent disconnect could orphan another agent's link record.
    const summary = await mgr.disconnect({
      serverName: BROWSEROS_MCP_SERVER_NAME,
      agent: agentId,
      removeIfLast: true,
    })
    logger.info('disconnected browseros from harness', {
      harness,
      agent: agentId,
      unlinked: summary.unlinked,
      removedManifest: summary.removedManifest,
    })
    return {
      harness,
      installed: false,
      agentId,
      message: `BrowserOS unregistered from ${harness}.`,
    }
  } catch (err) {
    return failure(harness, agentId, err, 'disconnect')
  }
}

/**
 * One row per supported harness that is ACTUALLY installed on this
 * machine. Filters out any harness the library reports as
 * uninstalled via `isInstalled` (the same signal `link` gates on
 * throwing `AgentNotInstalledError`), so the UI never offers a
 * Connect button that would throw. Harnesses that already carry a
 * BrowserClaw link record are kept regardless of the current
 * `isInstalled` reading: if a link exists we already have a working
 * install, and the file will be there on disk to prove it.
 *
 * `listLinks` remains the authoritative source for `installed=true`;
 * a harness is `installed` iff a link record for
 * `(serverName: "BrowserClaw", agent: <id>)` exists.
 */
export async function listBrowserosConnections(): Promise<ConnectionState[]> {
  const mgr = getMcpManager()
  const agentIds = ALL_HARNESSES.map((h) => HARNESS_TO_AGENT_ID[h])

  let links: Awaited<ReturnType<typeof mgr.listLinks>> = []
  try {
    links = await mgr.listLinks({
      serverNames: [BROWSEROS_MCP_SERVER_NAME],
    })
  } catch (err) {
    logger.warn('listBrowserosConnections listLinks failed', {
      error: err instanceof Error ? err.message : String(err),
    })
    // Fall through: every harness reports not-installed. The
    // installed-agents gate below still runs so the list survives
    // a listLinks fault.
  }

  let installedMap: Awaited<ReturnType<typeof mgr.isInstalled>> = {}
  try {
    installedMap = await mgr.isInstalled({ agents: agentIds })
  } catch (err) {
    logger.warn('listBrowserosConnections isInstalled failed', {
      error: err instanceof Error ? err.message : String(err),
    })
    // If the install probe throws, default every agent to installed
    // so we do not silently hide the whole list on a transient
    // filesystem hiccup.
    for (const id of agentIds) installedMap[id] = true
  }

  const byAgent = new Map<AgentId, (typeof links)[number]>()
  for (const link of links) {
    byAgent.set(link.agent, link)
  }

  const rows: ConnectionState[] = []
  for (const harness of ALL_HARNESSES) {
    const agentId = HARNESS_TO_AGENT_ID[harness]
    const link = byAgent.get(agentId)
    const isInstalledOnDisk = installedMap[agentId] ?? false
    // Already-linked wins: an existing link means we have a working
    // install even if isInstalled dropped for a transient reason.
    if (!link && !isInstalledOnDisk) continue
    if (link) {
      rows.push({
        harness,
        installed: true,
        agentId,
        configPath: tildifyHomePath(link.configPath),
        message: `Configured in ${harness}.`,
      })
    } else {
      rows.push({
        harness,
        installed: false,
        agentId,
        message: `${harness} is not configured.`,
      })
    }
  }
  return rows
}

function failure(
  harness: Harness,
  agentId: AgentId,
  err: unknown,
  op: 'connect' | 'disconnect',
): ConnectionState {
  if (err instanceof AgentNotInstalledError) {
    logger.info('browseros connect target not installed', {
      harness,
      agent: err.agent,
      configPath: err.configPath,
      parentDir: err.parentDir,
    })
    return {
      harness,
      installed: false,
      agentId,
      message: `${harness} is not installed on this machine. Launch it once so the MCP config directory exists, then try again.`,
    }
  }
  if (err instanceof ForeignEntryError) {
    logger.warn('browseros harness entry exists but was not written by us', {
      harness,
      serverName: err.serverName,
      agent: err.agent,
      configPath: err.configPath,
    })
    return {
      harness,
      installed: false,
      agentId,
      configPath: tildifyHomePath(err.configPath),
      message: `${harness} already has an entry under "${err.serverName}" that we did not write. Remove it from the config and try again.`,
    }
  }
  const message = err instanceof Error ? err.message : String(err)
  logger.warn('browseros connect operation failed', {
    harness,
    op,
    error: message,
  })
  return {
    harness,
    installed: op === 'disconnect',
    agentId,
    message: `Could not ${op} ${harness}: ${message}`,
  }
}
