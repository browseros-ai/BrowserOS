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
  /**
   * Server-issued turn id, set as soon as the response headers arrive
   * (`X-Turn-Id`) for fresh sends, or from the active-turn payload on
   * resume. Required for the historic-files fallback fetch; absent on
   * the brief optimistic window before the first header.
   */
  turnId?: string | null
  userText: string
  userAttachments?: UserAttachmentPreview[]
  parts: AssistantPart[]
  done: boolean
  timestamp: number
}
