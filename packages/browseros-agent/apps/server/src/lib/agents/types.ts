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

/**
 * One file the harness attributed to the assistant turn that just
 * finished. Emitted as part of a `produced_files` event before the
 * terminal `done` so the inline artifact card renders alongside the
 * streamed text the user just watched complete.
 */
export interface ProducedFileEventEntry {
  id: string
  /** Workspace-relative POSIX path. */
  path: string
  size: number
  mtimeMs: number
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
      type: 'produced_files'
      files: ProducedFileEventEntry[]
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

/**
 * Per-agent metadata sourced from the acpx session record. Surfaced
 * by the listing endpoint to fill in command-center row info that the
 * standard `getHistory` shape doesn't carry (cwd, token usage, last
 * user message). Returned `null` when the agent has no record yet.
 */
export interface AgentRowSnapshot {
  cwd: string | null
  lastUsedAt: number | null
  lastUserMessage: string | null
  tokens: {
    cumulative: { input: number; output: number }
    /**
     * 7-day rolling tokens. Zeroes today; populated in a follow-up that
     * tracks per-turn deltas in an activity ledger (the session record
     * doesn't carry per-message timestamps, so we can't bucket
     * accurately from it alone).
     */
    last7d: { input: number; output: number; requestCount: number }
  } | null
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
  /**
   * Optional. When present, the harness includes the snapshot fields
   * in `listAgentsWithActivity` for the command-center rows. Test
   * fakes can omit it; callers must tolerate `null`.
   */
  getRowSnapshot?(input: {
    agent: AgentDefinition
    sessionId: 'main'
  }): Promise<AgentRowSnapshot | null>
}
