/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  AgentDefinition,
  AgentHistoryEntry,
  AgentPermissionMode,
} from './agent-types'

export interface AgentStatus {
  state: 'ready' | 'unknown' | 'error'
  message?: string
}

export interface AgentSession {
  agentId: string
  id: 'main'
  updatedAt: number
}

export interface AgentHistoryPage {
  agentId: string
  sessionId: 'main'
  items: AgentHistoryEntry[]
}

export type AgentStreamEvent =
  | {
      type: 'text_delta'
      text: string
      stream: 'output' | 'thought'
      rawType?: string
    }
  | {
      type: 'tool_call'
      text: string
      title: string
      id?: string
      status?: string
      rawType?: string
    }
  | {
      type: 'status'
      text: string
      rawType?: string
    }
  | {
      type: 'done'
      text?: string
      stopReason?: string
    }
  | {
      type: 'error'
      message: string
      code?: string
    }

/**
 * Inline image attachment forwarded to the ACP `prompt` request as an
 * `image` content block. `data` is raw base64 (no `data:` prefix).
 */
export interface AgentInlineImage {
  mediaType: string
  data: string
}

export interface AgentPromptInput {
  agent: AgentDefinition
  sessionId: 'main'
  sessionKey: string
  message: string
  attachments?: ReadonlyArray<AgentInlineImage>
  permissionMode: AgentPermissionMode
  cwd?: string
  timeoutMs?: number
  signal?: AbortSignal
}

export interface AgentRuntime {
  status(agent: AgentDefinition): Promise<AgentStatus>
  listSessions(agent: AgentDefinition): Promise<AgentSession[]>
  getHistory(input: {
    agent: AgentDefinition
    sessionId: 'main'
  }): Promise<AgentHistoryPage>
  send(input: AgentPromptInput): Promise<ReadableStream<AgentStreamEvent>>
  cancel?(input: {
    agent: AgentDefinition
    sessionId: 'main'
    reason?: string
  }): Promise<void>
}
