/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export type AgentAdapter = 'claude' | 'codex' | 'openclaw' | 'hermes' | 'custom'

export type AgentPermissionMode = 'approve-all'

export interface AgentDefinition {
  id: string
  name: string
  adapter: AgentAdapter
  modelId?: string
  reasoningEffort?: string
  permissionMode: AgentPermissionMode
  sessionKey: string
  createdAt: number
  updatedAt: number
  /** Pinned agents float to the top of the rail. Defaulted on read for legacy records. */
  pinned?: boolean
  /** Command to launch the ACP binary. Required when adapter='custom'. */
  customCommand?: string
  /** Arguments passed to the custom ACP binary. */
  customArgs?: string[]
  /** Human-facing display name (spaces, emojis ok). Falls back to `name` when unset. */
  customLabel?: string
}

export interface AgentAdapterDescriptor {
  id: AgentAdapter
  name: string
  defaultModelId: string
  defaultReasoningEffort: string
  modelControl: 'runtime-supported' | 'best-effort'
  models: Array<{
    id: string
    label: string
    recommended?: boolean
  }>
  reasoningEfforts: Array<{
    id: string
    label: string
    recommended?: boolean
  }>
}

export interface AgentHistoryReasoning {
  text: string
  durationMs?: number
}

export interface AgentHistoryToolCall {
  toolCallId?: string
  toolName: string
  status: 'pending' | 'running' | 'completed' | 'failed'
  input?: unknown
  output?: unknown
  error?: string
  durationMs?: number
}

export interface AgentHistoryEntry {
  id: string
  agentId: string
  sessionId: 'main'
  role: 'user' | 'assistant'
  text: string
  createdAt: number
  reasoning?: AgentHistoryReasoning
  toolCalls?: AgentHistoryToolCall[]
}
