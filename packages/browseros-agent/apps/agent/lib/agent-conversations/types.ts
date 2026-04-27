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
  label: string
  subject?: string
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

/**
 * Attachments rendered alongside the user's text on the optimistic turn
 * — populated when the composer staged any images/files. The dataUrl is
 * the same one the server received; we keep it in memory only for the
 * lifetime of the live turn (history reload re-fetches via the JSONL).
 */
export interface UserAttachmentPreview {
  id: string
  kind: 'image' | 'file'
  mediaType: string
  name: string
  dataUrl?: string
}

export interface AgentConversationTurn {
  id: string
  userText: string
  userAttachments?: UserAttachmentPreview[]
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
