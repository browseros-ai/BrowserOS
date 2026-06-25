import type { AgentConversationTurn } from '@/lib/agent-conversations/types'

export interface AgentChatHistoryMessage {
  role: 'user' | 'assistant'
  content: string
}

export type AgentChatRole = 'user' | 'assistant'

export type AgentChatSource =
  | 'user-chat'
  | 'cron'
  | 'hook'
  | 'channel'
  | 'other'

export interface BrowserOSAgentSession {
  key: string
  updatedAt: number
  sessionId: string
  agentId: string
  kind: string
  source: AgentChatSource
  status?: string
  totalTokens?: number
  model?: string
  modelProvider?: string
}

export interface BrowserOSChatHistoryToolCall {
  toolCallId?: string
  toolName: string
  label: string
  subject?: string
  status: 'pending' | 'running' | 'completed' | 'failed'
  input?: unknown
  output?: unknown
  error?: string
  durationMs?: number
}

export interface BrowserOSChatHistoryReasoning {
  text: string
  durationMs?: number
}

export interface BrowserOSChatHistoryAttachment {
  kind: 'image' | 'file'
  mediaType: string
  // Images carry a `data:` URL so we can render directly without any
  // additional fetch; files (text/PDF) currently round-trip via inline
  // text in the message body and do not populate this field in v1.
  dataUrl?: string
  name?: string
}

export interface BrowserOSChatHistoryItem {
  id: string
  role: AgentChatRole
  text: string
  timestamp?: number
  messageSeq: number
  sessionKey: string
  source: AgentChatSource
  costUsd?: number
  tokensIn?: number
  tokensOut?: number
  toolCalls?: BrowserOSChatHistoryToolCall[]
  reasoning?: BrowserOSChatHistoryReasoning
  attachments?: BrowserOSChatHistoryAttachment[]
}

export interface AgentHistoryPageResponse {
  agentId: string
  sessionKey: string | null
  session: BrowserOSAgentSession | null
  items: BrowserOSChatHistoryItem[]
  page: {
    cursor?: string
    hasMore: boolean
    limit: number
  }
}

export type AgentChatMessageStatus =
  | 'historical'
  | 'sending'
  | 'streaming'
  | 'error'

export type AgentChatMessagePart =
  | { type: 'text'; text: string }
  | { type: 'reasoning'; text: string; duration?: number }
  | {
      type: 'tool-call'
      name: string
      label: string
      subject?: string
      status: 'pending' | 'running' | 'completed' | 'failed'
      input?: unknown
      output?: unknown
      error?: string
      durationMs?: number
    }
  | {
      type: 'attachment'
      kind: 'image' | 'file'
      mediaType: string
      dataUrl?: string
      name?: string
    }
  | { type: 'meta'; label: string; value: string }

export interface AgentChatMessage {
  id: string
  role: AgentChatRole
  sessionKey: string
  timestamp?: number
  source?: AgentChatSource
  messageSeq?: number
  status?: AgentChatMessageStatus
  parts: AgentChatMessagePart[]
  costUsd?: number
  tokensIn?: number
  tokensOut?: number
}

export function mapHistoryItemToAgentMessage(
  item: BrowserOSChatHistoryItem,
): AgentChatMessage {
  const parts: AgentChatMessagePart[] = []

  // Attachments first — they belong above the text in user messages and
  // never appear on assistant messages today (assistant images come back
  // through tool results, which render via the Task collapsible).
  if (item.attachments && item.attachments.length > 0) {
    for (const attachment of item.attachments) {
      parts.push({
        type: 'attachment',
        kind: attachment.kind,
        mediaType: attachment.mediaType,
        dataUrl: attachment.dataUrl,
        name: attachment.name,
      })
    }
  }

  // Reasoning, then tool calls, then text — the chronological order the
  // agent produced them (think → act → answer).
  if (item.reasoning && item.reasoning.text.trim().length > 0) {
    // 0ms means thinking and the final answer were emitted in the same JSONL
    // line (no tool calls between them) — there's no real elapsed wall-clock,
    // so fall through to the "Thinking" trigger instead of "Thought for 0
    // seconds" / streaming shimmer. Real multi-line turns floor at 1s.
    const durationMs = item.reasoning.durationMs ?? 0
    const duration =
      durationMs > 0 ? Math.max(1, Math.round(durationMs / 1000)) : undefined
    parts.push({
      type: 'reasoning',
      text: item.reasoning.text,
      duration,
    })
  }

  if (item.toolCalls && item.toolCalls.length > 0) {
    for (const tc of item.toolCalls) {
      parts.push({
        type: 'tool-call',
        name: tc.toolName,
        label: tc.label,
        subject: tc.subject,
        status: tc.status,
        input: tc.input,
        output: tc.output,
        error: tc.error,
        durationMs: tc.durationMs,
      })
    }
  }

  // Only emit a text part when there's actual content. User messages with
  // only attachments and no caption shouldn't render an empty bubble.
  if (item.text.trim().length > 0) {
    parts.push({ type: 'text', text: item.text })
  }

  return {
    id: item.id,
    role: item.role,
    sessionKey: item.sessionKey,
    timestamp: item.timestamp,
    source: item.source,
    messageSeq: item.messageSeq,
    status: 'historical',
    parts,
    costUsd: item.costUsd,
    tokensIn: item.tokensIn,
    tokensOut: item.tokensOut,
  }
}

export function flattenHistoryPages(
  pages: AgentHistoryPageResponse[],
): AgentChatMessage[] {
  return pages
    .flatMap((page) => page.items)
    .sort((a, b) => {
      if (a.timestamp != null && b.timestamp != null) {
        return a.timestamp - b.timestamp
      }
      return a.messageSeq - b.messageSeq
    })
    .map(mapHistoryItemToAgentMessage)
}

export function buildChatHistoryFromAgentMessages(
  messages: AgentChatMessage[],
): AgentChatHistoryMessage[] {
  return messages
    .map((message) => {
      const content = message.parts
        .filter((part): part is { type: 'text'; text: string } => {
          return part.type === 'text' && part.text.trim().length > 0
        })
        .map((part) => part.text.trim())
        .join('\n\n')

      return content ? { role: message.role, content } : null
    })
    .filter((message): message is AgentChatHistoryMessage => Boolean(message))
}

const TURN_HISTORY_MATCH_WINDOW_MS = 5_000

export function filterTurnsPersistedInHistory(
  turns: AgentConversationTurn[],
  historyMessages: AgentChatMessage[],
): AgentConversationTurn[] {
  return turns.filter(
    (turn) => !isTurnPersistedInHistory(turn, historyMessages),
  )
}

function isTurnPersistedInHistory(
  turn: AgentConversationTurn,
  historyMessages: AgentChatMessage[],
): boolean {
  if (!turn.done) return false

  const assistantText = getTurnAssistantText(turn)
  if (!assistantText) return false

  const minTimestamp = turn.timestamp - TURN_HISTORY_MATCH_WINDOW_MS
  const userText = turn.userText.trim()
  const userPersisted =
    !userText ||
    historyMessages.some(
      (message) =>
        message.role === 'user' &&
        isHistoryMessageAfter(message, minTimestamp) &&
        getAgentMessageText(message) === userText,
    )
  const assistantPersisted = historyMessages.some(
    (message) =>
      message.role === 'assistant' &&
      isHistoryMessageAfter(message, minTimestamp) &&
      getAgentMessageText(message) === assistantText,
  )

  return userPersisted && assistantPersisted
}

function isHistoryMessageAfter(
  message: AgentChatMessage,
  minTimestamp: number,
): boolean {
  return message.timestamp == null || message.timestamp >= minTimestamp
}

function getTurnAssistantText(turn: AgentConversationTurn): string {
  return turn.parts
    .filter((part) => part.kind === 'text')
    .map((part) => part.text)
    .join('')
    .trim()
}

function getAgentMessageText(message: AgentChatMessage): string {
  return message.parts
    .filter((part) => part.type === 'text')
    .map((part) => part.text)
    .join('')
    .trim()
}
