export interface HistoryConversation {
  id: string
  lastMessagedAt: number
  lastUserMessage: string
}

export type TimeGroup = 'today' | 'thisWeek' | 'thisMonth' | 'older'

export interface GroupedConversations {
  today: HistoryConversation[]
  thisWeek: HistoryConversation[]
  thisMonth: HistoryConversation[]
  older: HistoryConversation[]
}
