/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { AgentDefinition, AgentSessionId } from '../agent-types'
import { prepareClaudeCodeContext, prepareCodexContext } from '../runtime'

export interface PreparedAcpxAgentContext {
  cwd: string
  runtimeSessionKey: string
  runPrompt: string
  commandEnv: Record<string, string>
  commandIdentity: string
  useBrowserosMcp: boolean
  /**
   * Hostname the agent should use to reach the BrowserOS HTTP MCP server.
   * Default `127.0.0.1` is correct for host-process adapters.
   */
  browserosMcpHost?: string
}

export interface PrepareAcpxAgentContextInput {
  browserosDir: string
  agent: AgentDefinition
  sessionId: AgentSessionId
  sessionKey: string
  cwdOverride: string | null
  isSelectedCwd: boolean
  message: string
}

interface AcpxAgentAdapter {
  prepare(
    input: PrepareAcpxAgentContextInput,
  ): Promise<PreparedAcpxAgentContext>
}

const ADAPTERS: Record<AgentDefinition['adapter'], AcpxAgentAdapter> = {
  claude: { prepare: prepareClaudeCodeContext },
  codex: { prepare: prepareCodexContext },
}

function getAcpxAgentAdapter(
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
