import type { OpenClawChatHistoryMessage } from '@/entrypoints/app/agents/useOpenClaw'
import type { AgentConversationTurn } from '@/lib/agent-conversations/types'
import type { ProducedFilesRailGroup } from '@/lib/agent-files'

export type ClawChatRole = 'user' | 'assistant'

export type ClawChatSource = 'user-chat' | 'cron' | 'hook' | 'channel' | 'other'

export interface BrowserOSOpenClawSession {
  key: string
  updatedAt: number
  sessionId: string
  agentId: string
  kind: string
  source: ClawChatSource
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
  role: ClawChatRole
  text: string
  timestamp?: number
  messageSeq: number
  sessionKey: string
  source: ClawChatSource
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
  session: BrowserOSOpenClawSession | null
  items: BrowserOSChatHistoryItem[]
  page: {
    cursor?: string
    hasMore: boolean
    limit: number
  }
}

export type ClawChatMessageStatus =
  | 'historical'
  | 'sending'
  | 'streaming'
  | 'error'

export type ClawChatMessagePart =
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

export interface ClawChatMessage {
  id: string
  role: ClawChatRole
  sessionKey: string
  timestamp?: number
  source?: ClawChatSource
  messageSeq?: number
  status?: ClawChatMessageStatus
  parts: ClawChatMessagePart[]
  costUsd?: number
  tokensIn?: number
  tokensOut?: number
}

export function mapHistoryItemToClawMessage(
  item: BrowserOSChatHistoryItem,
): ClawChatMessage {
  const parts: ClawChatMessagePart[] = []

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
): ClawChatMessage[] {
  return pages
    .flatMap((page) => page.items)
    .sort((a, b) => {
      if (a.timestamp != null && b.timestamp != null) {
        return a.timestamp - b.timestamp
      }
      return a.messageSeq - b.messageSeq
    })
    .map(mapHistoryItemToClawMessage)
}

export function buildChatHistoryFromClawMessages(
  messages: ClawChatMessage[],
): OpenClawChatHistoryMessage[] {
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
    .filter((message): message is OpenClawChatHistoryMessage =>
      Boolean(message),
    )
}

const TURN_HISTORY_MATCH_WINDOW_MS = 5_000

export function filterTurnsPersistedInHistory(
  turns: AgentConversationTurn[],
  historyMessages: ClawChatMessage[],
): AgentConversationTurn[] {
  return turns.filter(
    (turn) => !isTurnPersistedInHistory(turn, historyMessages),
  )
}

/**
 * Persisted turns that still carry `producedFiles` — once history
 * reloads, the assistant text is rendered by `ClawChatMessage` and
 * the optimistic turn is filtered out by
 * `filterTurnsPersistedInHistory`. The historical message has no
 * `producedFiles` field (history items don't carry that), so the
 * inline file-card strip would vanish on history reload.
 *
 * Returning these here lets the caller render a strip-only entry
 * after the corresponding history bubble — full message stays as
 * the persisted history pair, but the produced-files affordance
 * survives.
 */
export function selectStripOnlyTurns(
  turns: AgentConversationTurn[],
  historyMessages: ClawChatMessage[],
): AgentConversationTurn[] {
  return turns.filter(
    (turn) =>
      Boolean(turn.producedFiles && turn.producedFiles.length > 0) &&
      isTurnPersistedInHistory(turn, historyMessages),
  )
}

function isTurnPersistedInHistory(
  turn: AgentConversationTurn,
  historyMessages: ClawChatMessage[],
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
        getClawMessageText(message) === userText,
    )
  const assistantPersisted = historyMessages.some(
    (message) =>
      message.role === 'assistant' &&
      isHistoryMessageAfter(message, minTimestamp) &&
      getClawMessageText(message) === assistantText,
  )

  return userPersisted && assistantPersisted
}

function isHistoryMessageAfter(
  message: ClawChatMessage,
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

function getClawMessageText(message: ClawChatMessage): string {
  return message.parts
    .filter((part) => part.type === 'text')
    .map((part) => part.text)
    .join('')
    .trim()
}

function firstNonBlankLine(value: string): string {
  for (const raw of value.split('\n')) {
    const trimmed = raw.trim()
    if (trimmed) return trimmed
  }
  return ''
}

/**
 * Map each assistant history message to the produced-files group
 * that came from its turn. Match key is `group.turnPrompt` (first
 * non-blank line of the user prompt that initiated the turn) vs.
 * the first non-blank line of the user message that immediately
 * preceded this assistant message — the same shape the server
 * emits when storing turnPrompt.
 *
 * Walks history forward (oldest-first per `flattenHistoryPages`)
 * and consumes groups in chronological order. A group can only
 * match once — if two turns share the same prompt the earlier
 * one wins, and the later assistant message stays unassociated
 * (those land back in `tailStripGroups` at the conversation tail).
 */
export function mapHistoryToProducedFilesGroups(
  historyMessages: ClawChatMessage[],
  groups: ReadonlyArray<ProducedFilesRailGroup>,
): {
  byAssistantMessageId: Map<string, ProducedFilesRailGroup>
  unmatched: ProducedFilesRailGroup[]
} {
  const byAssistantMessageId = new Map<string, ProducedFilesRailGroup>()
  if (groups.length === 0) {
    return { byAssistantMessageId, unmatched: [] }
  }
  // Oldest-first so the iteration order matches history.
  const remaining = [...groups].sort((a, b) => a.createdAt - b.createdAt)

  let pendingPrompt: string | null = null
  for (const message of historyMessages) {
    if (message.role === 'user') {
      pendingPrompt = firstNonBlankLine(getClawMessageText(message))
      continue
    }
    if (message.role !== 'assistant' || !pendingPrompt) continue
    const matchIndex = remaining.findIndex(
      (group) => group.turnPrompt === pendingPrompt,
    )
    if (matchIndex >= 0) {
      const [match] = remaining.splice(matchIndex, 1)
      byAssistantMessageId.set(message.id, match)
    }
    pendingPrompt = null
  }

  return { byAssistantMessageId, unmatched: remaining }
}
