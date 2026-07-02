/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 single-endpoint install layer. Writes one canonical
 * `"browseros"` entry into the harness's MCP config file via
 * `agent-mcp-manager`, pointing at the slugless canonical URL
 * (`http://127.0.0.1:9200/mcp`). One row per supported
 * harness, idempotent connect / disconnect, list reads through the
 * library's manifest so the UI reflects the current install state
 * within the polling interval.
 *
 * Distinct from `harness-install.ts` (the legacy per-agent path).
 * That path writes one entry per cockpit agent profile, keyed by the
 * profile's slug, with a slug-shaped URL. v2 has no per-agent
 * profile, so this layer writes one entry keyed by the constant
 * `BROWSEROS_MCP_SERVER_NAME` ("BrowserClaw") with the slugless URL.
 * Both layers share `specFor` for transport selection (HTTP vs the
 * stdio `npx mcp-remote` fallback for stdio-only agents).
 */

import type { AgentId } from 'agent-mcp-manager'
import { ForeignEntryError } from 'agent-mcp-manager'
import { env } from '../env'
import { logger } from '../lib/logger'
import { getMcpManager } from '../lib/mcp-manager'
import { type Harness, harnessEnum } from '../routes/agents/schemas'
import {
  BROWSEROS_MCP_SERVER_NAME,
  canonicalMcpUrlForPort,
} from '../shared/mcp-url'
import { HARNESS_TO_AGENT_ID } from './harness-install'
import { relinkManagedServer } from './mcp-relink'
import { specFor } from './spec-for'

export interface ConnectionState {
  harness: Harness
  /**
   * True when the harness has a "browseros" entry pointing at the
   * canonical URL. For BrowserOS-internal harnesses (Hermes,
   * OpenClaw) `installed` is always true (no third-party config to
   * write).
   */
  installed: boolean
  /** Filled when `installed` is true and a real config file was touched. */
  configPath?: string
  /** Stable agent-mcp-manager id; null for BrowserOS-internal harnesses. */
  agentId: AgentId | null
  /** Single-line human-readable message; surfaced to the UI verbatim. */
  message: string
}

const ALL_HARNESSES: readonly Harness[] = harnessEnum.options

function canonicalMcpUrl(): string {
  return canonicalMcpUrlForPort(env.proxyPort ?? env.port)
}

export async function connectBrowserosToHarness(
  harness: Harness,
): Promise<ConnectionState> {
  const agentId = HARNESS_TO_AGENT_ID[harness]
  if (agentId === null) {
    return {
      harness,
      installed: true,
      agentId: null,
      message: `${harness} runs inside BrowserOS; no harness config to write.`,
    }
  }
  const mgr = getMcpManager()
  const url = canonicalMcpUrl()
  try {
    const link = await relinkManagedServer({
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
    logger.info('connected browseros to harness', {
      harness,
      agent: agentId,
      configPath: link.configPath,
    })
    return {
      harness,
      installed: true,
      agentId,
      configPath: link.configPath,
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
  if (agentId === null) {
    return {
      harness,
      installed: false,
      agentId: null,
      message: `${harness} runs inside BrowserOS; nothing to disconnect.`,
    }
  }
  const mgr = getMcpManager()
  try {
    const unlink = await mgr.unlink({
      serverName: BROWSEROS_MCP_SERVER_NAME,
      agent: agentId,
    })
    // Only drop the shared manifest entry when NO other agents are
    // still linked to it. The BrowserClaw server is a single manifest
    // record that agent-mcp-manager fans out across every agent's
    // config file; unconditionally calling remove() here would wipe
    // the shared entry and orphan every other agent's on-disk link.
    // listLinks after unlink is safe: the library queues writes, so
    // this read sees the post-unlink state.
    try {
      const remainingLinks = await mgr.listLinks({
        serverNames: [BROWSEROS_MCP_SERVER_NAME],
      })
      if (remainingLinks.length === 0) {
        await mgr.remove({
          serverName: BROWSEROS_MCP_SERVER_NAME,
          unlinkFirst: false,
        })
      }
    } catch {
      // ServerNotFoundError, etc. Safe to ignore: the link is gone,
      // which is the user-visible state we care about.
    }
    logger.info('disconnected browseros from harness', {
      harness,
      agent: agentId,
      configPath: unlink.configPath,
    })
    return {
      harness,
      installed: false,
      agentId,
      configPath: unlink.configPath,
      message: `BrowserOS unregistered from ${harness}.`,
    }
  } catch (err) {
    return failure(harness, agentId, err, 'disconnect')
  }
}

/**
 * One row per supported harness. The library's `listLinks` is the
 * authoritative source: a harness is `installed` iff a link record
 * for `(serverName: "browseros", agent: <id>)` exists. Internal
 * harnesses always report `installed: true` so the UI badge counts
 * them correctly.
 */
export async function listBrowserosConnections(): Promise<ConnectionState[]> {
  const mgr = getMcpManager()
  let links: Awaited<ReturnType<typeof mgr.listLinks>> = []
  try {
    links = await mgr.listLinks({
      serverNames: [BROWSEROS_MCP_SERVER_NAME],
    })
  } catch (err) {
    logger.warn('listBrowserosConnections failed', {
      error: err instanceof Error ? err.message : String(err),
    })
    // Fall through: every external harness reports not-installed.
  }
  const byAgent = new Map<AgentId, (typeof links)[number]>()
  for (const link of links) {
    if (!link.broken) byAgent.set(link.agent, link)
  }
  return ALL_HARNESSES.map((harness): ConnectionState => {
    const agentId = HARNESS_TO_AGENT_ID[harness]
    if (agentId === null) {
      return {
        harness,
        installed: true,
        agentId: null,
        message: `${harness} runs inside BrowserOS.`,
      }
    }
    const link = byAgent.get(agentId)
    if (link) {
      return {
        harness,
        installed: true,
        agentId,
        configPath: link.configPath,
        message: `Configured in ${harness}.`,
      }
    }
    return {
      harness,
      installed: false,
      agentId,
      message: `${harness} is not configured.`,
    }
  })
}

function failure(
  harness: Harness,
  agentId: AgentId,
  err: unknown,
  op: 'connect' | 'disconnect',
): ConnectionState {
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
      configPath: err.configPath,
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
