/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export type AgentAdapter = 'claude' | 'codex' | 'openclaw'

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
