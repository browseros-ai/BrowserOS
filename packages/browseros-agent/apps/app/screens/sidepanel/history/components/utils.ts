import type { UIMessage } from 'ai'
import dayjs from 'dayjs'
import type {
  GroupedConversations,
  HistoryConversation,
  TimeGroup,
} from './types'

export const TIME_GROUP_LABELS: Record<TimeGroup, string> = {
  today: 'Today',
  thisWeek: 'This Week',
  thisMonth: 'This Month',
  older: 'Older',
}

const getTimeGroup = (timestamp: number): TimeGroup => {
  const date = dayjs(timestamp)
  const now = dayjs()

  if (date.isSame(now, 'day')) return 'today'
  if (date.isSame(now, 'week')) return 'thisWeek'
  if (date.isSame(now, 'month')) return 'thisMonth'
  return 'older'
}

export const extractLastUserMessage = (messages: UIMessage[]): string => {
  const userMessages = messages.filter((m) => m.role === 'user')
  const lastUserMessage = userMessages[userMessages.length - 1]

  if (!lastUserMessage) return 'New conversation'

  const textParts = lastUserMessage.parts.filter((p) => p.type === 'text')
  const text = textParts.map((p) => (p as { text: string }).text).join(' ')

  return text || 'New conversation'
}

/**
 * Merges local and remote history by id, newest first. The local copy wins for
 * conversations we still hold locally (its lastMessagedAt is the message time);
 * remote fills in older, cloud-only history. Local id == remote rowId makes the
 * dedupe exact (issue #559).
 */
export const mergeHistoryConversations = (
  local: HistoryConversation[],
  remote: HistoryConversation[],
): HistoryConversation[] => {
  const byId = new Map<string, HistoryConversation>()
  for (const conversation of remote) byId.set(conversation.id, conversation)
  for (const conversation of local) byId.set(conversation.id, conversation)
  return [...byId.values()].sort((a, b) => b.lastMessagedAt - a.lastMessagedAt)
}

export const groupConversations = (
  conversations: HistoryConversation[],
): GroupedConversations => {
  const groups: GroupedConversations = {
    today: [],
    thisWeek: [],
    thisMonth: [],
    older: [],
  }

  for (const conversation of conversations) {
    const group = getTimeGroup(conversation.lastMessagedAt)
    groups[group].push(conversation)
  }

  return groups
}
