/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Wires a stored cockpit agent profile into the user's chosen harness
 * MCP config via `agent-mcp-manager`. Install and uninstall are
 * best-effort from profile persistence: failed harness writes return
 * an outcome instead of rolling back the profile mutation.
 */

import type { AgentId } from 'agent-mcp-manager'
import {
  AgentNotSupportedError,
  ForeignEntryError,
  resolveAgentMcpConfigPath,
} from 'agent-mcp-manager'
import { logger } from '../lib/logger'
import { getMcpManager } from '../lib/mcp-manager'
import type { Harness, StoredAgentProfile } from '../routes/agents/schemas'
import { relinkManagedServer } from './mcp-relink'
import { specFor } from './spec-for'

export interface InstallOutcome {
  installed: boolean
  message: string
  agent?: AgentId
  configPath?: string
}

/** Maps stored harness labels to the upstream agent-mcp-manager id. */
export const HARNESS_TO_AGENT_ID: Record<Harness, AgentId> = {
  'Claude Code': 'claude-code',
  Codex: 'codex',
  Cursor: 'cursor',
  OpenCode: 'opencode',
  Antigravity: 'antigravity',
  'VS Code': 'vscode',
  Zed: 'zed',
}

export async function installForAgent(
  profile: Pick<StoredAgentProfile, 'slug' | 'mcpUrl' | 'harness'>,
): Promise<InstallOutcome> {
  const agentId = HARNESS_TO_AGENT_ID[profile.harness]
  const mgr = getMcpManager()
  const spec = specFor(agentId, profile.mcpUrl)
  try {
    await relinkManagedServer({
      mgr,
      serverName: profile.slug,
      agent: agentId,
      spec,
    })
    const configPath = await resolveAgentMcpConfigPath(agentId, 'system').catch(
      () => undefined,
    )
    logger.info('installed cockpit agent into harness', {
      slug: profile.slug,
      agent: agentId,
      configPath,
    })
    return {
      installed: true,
      message: `Endpoint registered with ${profile.harness}.`,
      agent: agentId,
      configPath,
    }
  } catch (err) {
    return failure(err, profile.harness)
  }
}

export async function uninstallForAgent(
  profile: Pick<StoredAgentProfile, 'slug' | 'harness'>,
): Promise<InstallOutcome> {
  const agentId = HARNESS_TO_AGENT_ID[profile.harness]
  const mgr = getMcpManager()
  try {
    // `disconnect` is the 0.0.4 primitive that unlinks the agent AND
    // drops the manifest entry only when no other agents remain
    // linked to it. Replaces the pre-0.0.4 three-step unlink + list
    // + conditional remove dance, which had a race window where two
    // concurrent uninstalls could orphan each other's link records.
    await mgr.disconnect({
      serverName: profile.slug,
      agent: agentId,
      removeIfLast: true,
    })
    logger.info('uninstalled cockpit agent from harness', {
      slug: profile.slug,
      agent: agentId,
    })
    return {
      installed: false,
      message: `Endpoint unregistered from ${profile.harness}.`,
      agent: agentId,
    }
  } catch (err) {
    return failure(err, profile.harness)
  }
}

function failure(err: unknown, harness: Harness): InstallOutcome {
  if (err instanceof ForeignEntryError) {
    logger.warn('harness entry exists but was not written by us', {
      harness,
      serverName: err.serverName,
      agent: err.agent,
      configPath: err.configPath,
    })
    return {
      installed: false,
      message: `${harness} already has an entry under this name that we didn't write; remove it from the config and try again.`,
    }
  }
  if (err instanceof AgentNotSupportedError) {
    return {
      installed: false,
      message: `${harness} is not supported by the install layer (agent: ${err.agent}).`,
    }
  }
  const message = err instanceof Error ? err.message : String(err)
  logger.warn('harness install failed', { harness, error: message })
  return {
    installed: false,
    message: `Could not register endpoint with ${harness}: ${message}`,
  }
}
