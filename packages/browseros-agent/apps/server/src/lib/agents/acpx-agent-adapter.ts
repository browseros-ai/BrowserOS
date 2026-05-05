/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { createRuntimeStore } from 'acpx/runtime'
import type { OpenClawGatewayChatClient } from '../../api/services/openclaw/openclaw-gateway-chat-client'
import type { AgentDefinition } from './agent-types'
import { prepareClaudeCodeContext } from './claude-code/prepare'
import { prepareCodexContext } from './codex/prepare'
import {
  maybeHandleOpenClawTurn,
  prepareOpenClawContext,
} from './openclaw/prepare'
import type { AgentPromptInput, AgentStreamEvent } from './types'

export interface PreparedAcpxAgentContext {
  cwd: string
  runtimeSessionKey: string
  runPrompt: string
  commandEnv: Record<string, string>
  commandIdentity: string
  useBrowserosMcp: boolean
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

export interface AcpxAdapterTurnInput {
  prompt: AgentPromptInput
  prepared: PreparedAcpxAgentContext
  sessionStore: ReturnType<typeof createRuntimeStore>
  openclawGatewayChat: OpenClawGatewayChatClient | null
}

export interface AcpxAgentAdapter {
  prepare(
    input: PrepareAcpxAgentContextInput,
  ): Promise<PreparedAcpxAgentContext>
  maybeHandleTurn?(
    input: AcpxAdapterTurnInput,
  ): Promise<ReadableStream<AgentStreamEvent> | null>
}

const ADAPTERS: Record<AgentDefinition['adapter'], AcpxAgentAdapter> = {
  claude: { prepare: prepareClaudeCodeContext },
  codex: { prepare: prepareCodexContext },
  openclaw: {
    prepare: prepareOpenClawContext,
    maybeHandleTurn: maybeHandleOpenClawTurn,
  },
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
