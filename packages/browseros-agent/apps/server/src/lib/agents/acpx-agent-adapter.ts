/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AgentDefinition } from './agent-types'
import { prepareClaudeCodeContext } from './claude-code/prepare'
import { prepareCodexContext } from './codex/prepare'
import { prepareOpenClawContext } from './openclaw/prepare'
import { prepareHermesContext } from './runtime/hermes-container-runtime'

export interface PreparedAcpxAgentContext {
  cwd: string
  runtimeSessionKey: string
  runPrompt: string
  commandEnv: Record<string, string>
  commandIdentity: string
  useBrowserosMcp: boolean
  /**
   * Hostname the agent should use to reach the BrowserOS HTTP MCP server.
   * Default `127.0.0.1` is correct for host-process adapters (claude, codex,
   * Phase A host-mode hermes). Container-spawned adapters override this to
   * `host.containers.internal` so the URL injected into ACP newSession's
   * mcpServers resolves from inside the container.
   */
  browserosMcpHost?: string
  openclawSessionKey: string | null
}

export interface PrepareAcpxAgentContextInput {
  browserosDir: string
  agent: AgentDefinition
  sessionId: 'main'
  sessionKey: string
  cwdOverride: string | null
  isSelectedCwd: boolean
  message: string
}

export interface AcpxAgentAdapter {
  prepare(
    input: PrepareAcpxAgentContextInput,
  ): Promise<PreparedAcpxAgentContext>
}

const ADAPTERS: Record<AgentDefinition['adapter'], AcpxAgentAdapter> = {
  claude: { prepare: prepareClaudeCodeContext },
  codex: { prepare: prepareCodexContext },
  openclaw: { prepare: prepareOpenClawContext },
  hermes: { prepare: prepareHermesContext },
}

export function getAcpxAgentAdapter(
  adapter: AgentDefinition['adapter'],
): AcpxAgentAdapter {
  return ADAPTERS[adapter]
}

/** Prepares adapter-specific filesystem, prompt, env, and session identity for one ACPX turn. */
export async function prepareAcpxAgentContext(
  input: PrepareAcpxAgentContextInput,
): Promise<PreparedAcpxAgentContext> {
  return getAcpxAgentAdapter(input.agent.adapter).prepare(input)
}
