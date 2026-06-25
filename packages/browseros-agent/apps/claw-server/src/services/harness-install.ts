/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Wires a cockpit agent profile into the user's chosen harness's MCP
 * config file via `agent-mcp-manager`. Each cockpit profile becomes
 * one entry in the harness's config, keyed by the profile's slug,
 * pointing at `http://127.0.0.1:9200/mcp/<slug>`.
 *
 * `installForAgent` runs on POST /agents (right after the profile
 * file is written). `uninstallForAgent` runs on DELETE /agents/:id
 * (right before the profile file is removed). Both are best-effort
 * from the caller's point of view: an install failure does not
 * prevent the profile from being created, and an uninstall failure
 * does not prevent the profile from being deleted. The HTTP response
 * carries the outcome so the UI can surface it.
 *
 * Harness mapping is documented in HARNESS_TO_AGENT_ID below; two
 * harnesses (Hermes, OpenClaw) are BrowserOS-internal and do not
 * have a third-party config to write, so they short-circuit as a
 * no-op success.
 */

import type { AgentId } from 'agent-mcp-manager'
import { AgentNotSupportedError, ForeignEntryError } from 'agent-mcp-manager'
import { logger } from '../lib/logger'
import { getMcpManager } from '../lib/mcp-manager'
import type { Harness, StoredAgentProfile } from '../routes/agents/schemas'
import { specFor } from './spec-for'

export interface InstallOutcome {
  /** True iff the harness config was written successfully (or no-op for internal harnesses). */
  installed: boolean
  /**
   * Single-line human-readable message. Always present so the UI can
   * surface the same string for success and failure.
   */
  message: string
  /** Filled when `installed` is true and the library wrote to a real file. */
  agent?: AgentId
  configPath?: string
}

/**
 * Map the wizard's harness label to the upstream library's agent id.
 * `null` means "no third-party config to write" (BrowserOS-internal
 * harness); the install short-circuits as a successful no-op.
 *
 * If a mapping is wrong, change it here and every install/uninstall
 * path picks up the new target.
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
    // `add` overwrites any existing entry with the same name so URL
    // drift (port change between boots) gets caught on every install.
    await mgr.add({ name: profile.slug, spec })
    const link = await mgr.link({ serverName: profile.slug, agent: agentId })
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
 * Re-sync the harness MCP config after a profile mutation that
 * either rotated the slug or swapped the harness (or both). Always
 * installs the new (harness, slug) FIRST so the harness has a
 * working entry continuously; the stale entry under the old
 * (harness, slug) is unlinked afterwards.
 *
 * No-op when both the harness and slug stayed the same. Returns the
 * install + uninstall outcomes so callers can log them (today) or
 * surface them in the response (later).
 */
export async function reconcileHarnessLink(input: {
  before: Pick<StoredAgentProfile, 'slug' | 'harness'>
  after: Pick<StoredAgentProfile, 'slug' | 'mcpUrl' | 'harness'>
}): Promise<{
  install: InstallOutcome | null
  uninstall: InstallOutcome | null
}> {
  const { before, after } = input
  const harnessChanged = before.harness !== after.harness
  const slugChanged = before.slug !== after.slug
  if (!harnessChanged && !slugChanged) {
    return { install: null, uninstall: null }
  }
  const install = await installForAgent(after)
  const uninstall = await uninstallForAgent(before)
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
