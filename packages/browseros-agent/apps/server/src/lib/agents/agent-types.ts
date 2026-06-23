/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export type AgentAdapter = 'claude' | 'codex'

export type AgentPermissionMode = 'approve-all'

export const MAIN_AGENT_SESSION_ID = 'main'
export type AgentSessionId = string

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
  sessionId: AgentSessionId
  role: 'user' | 'assistant'
  text: string
  createdAt: number
  reasoning?: AgentHistoryReasoning
  toolCalls?: AgentHistoryToolCall[]
}
