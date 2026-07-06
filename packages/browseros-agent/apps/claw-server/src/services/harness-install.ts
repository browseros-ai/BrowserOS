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
import { AgentNotSupportedError, ForeignEntryError } from 'agent-mcp-manager'
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

/**
 * Maps stored harness labels to upstream agent ids; null means the
 * harness is BrowserOS-internal and has no third-party config file.
 */
export const HARNESS_TO_AGENT_ID: Record<Harness, AgentId | null> = {
  'Claude Code': 'claude-code',
  'Claude Desktop': 'claude-desktop',
  Cursor: 'cursor',
  'VS Code': 'vscode',
  Zed: 'zed',
  Codex: 'codex',
  'Gemini CLI': 'gemini',
  Hermes: null,
  OpenClaw: null,
}

export async function installForAgent(
  profile: Pick<StoredAgentProfile, 'slug' | 'mcpUrl' | 'harness'>,
): Promise<InstallOutcome> {
  const agentId = HARNESS_TO_AGENT_ID[profile.harness]
  if (agentId === null) {
    return {
      installed: true,
      message: `${profile.harness} runs inside BrowserOS; no harness config to write.`,
    }
  }
  const mgr = getMcpManager()
  const spec = specFor(agentId, profile.mcpUrl)
  try {
    const link = await relinkManagedServer({
      mgr,
      serverName: profile.slug,
      agent: agentId,
      spec,
    })
    logger.info('installed cockpit agent into harness', {
      slug: profile.slug,
      agent: agentId,
      configPath: link.configPath,
    })
    return {
      installed: true,
      message: `Endpoint registered with ${profile.harness}.`,
      agent: agentId,
      configPath: link.configPath,
    }
  } catch (err) {
    return failure(err, profile.harness)
  }
}

export async function uninstallForAgent(
  profile: Pick<StoredAgentProfile, 'slug' | 'harness'>,
): Promise<InstallOutcome> {
  const agentId = HARNESS_TO_AGENT_ID[profile.harness]
  if (agentId === null) {
    return {
      installed: false,
      message: `${profile.harness} runs inside BrowserOS; nothing to uninstall.`,
    }
  }
  const mgr = getMcpManager()
  try {
    await mgr.unlink({ serverName: profile.slug, agent: agentId })
    // Also drop the manifest entry so a future agent reusing the
    // slug isn't blocked by a lingering record.
    await mgr.remove({ serverName: profile.slug, unlinkFirst: false })
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

/**
 * Keeps the harness MCP entry aligned after profile slug, harness, or
 * URL changes.
 */
export async function reconcileHarnessLink(input: {
  before: Pick<StoredAgentProfile, 'slug' | 'mcpUrl' | 'harness'>
  after: Pick<StoredAgentProfile, 'slug' | 'mcpUrl' | 'harness'>
}): Promise<{
  install: InstallOutcome | null
  uninstall: InstallOutcome | null
}> {
  const { before, after } = input
  const harnessChanged = before.harness !== after.harness
  const slugChanged = before.slug !== after.slug
  const urlChanged = before.mcpUrl !== after.mcpUrl
  if (!harnessChanged && !slugChanged && !urlChanged) {
    return { install: null, uninstall: null }
  }
  const install = await installForAgent(after)
  const uninstall =
    harnessChanged || slugChanged ? await uninstallForAgent(before) : null
  return { install, uninstall }
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
