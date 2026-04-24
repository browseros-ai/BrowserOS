export interface AssistantTextPart {
  kind: 'text'
  text: string
}

export interface AssistantThinkingPart {
  kind: 'thinking'
  text: string
  done: boolean
}

export interface ToolEntry {
  id: string
  name: string
  status: 'running' | 'completed' | 'error'
  durationMs?: number
}

export interface AssistantToolBatchPart {
  kind: 'tool-batch'
  tools: ToolEntry[]
}

export type AssistantPart =
  | AssistantTextPart
  | AssistantThinkingPart
  | AssistantToolBatchPart

export interface AgentConversationTurn {
  id: string
  userText: string
  parts: AssistantPart[]
  done: boolean
  timestamp: number
}

export interface AgentConversation {
  agentId: string
  agentName: string
  sessionKey: string
  turns: AgentConversationTurn[]
  createdAt: number
  updatedAt: number
}

export interface AgentCardData {
  agentId: string
  name: string
  model?: string
  status: 'idle' | 'working' | 'error'
  lastMessage?: string
  lastMessageTimestamp?: number
  activitySummary?: string
  currentTool?: string
  costUsd?: number
}
