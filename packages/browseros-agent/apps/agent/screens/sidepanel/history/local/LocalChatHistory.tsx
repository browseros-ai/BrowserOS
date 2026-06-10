import type { FC } from 'react'
import { useMemo } from 'react'
import { useConversations } from '@/lib/conversations/useConversations'
import { useChatSessionContext } from '@/modules/chat/chat-session-context'
import { ConversationList } from '../components/ConversationList'
import type { HistoryConversation } from '../components/types'
import { extractLastUserMessage, groupConversations } from '../components/utils'

export const LocalChatHistory: FC = () => {
  const { conversations: localConversations, removeConversation } =
    useConversations()
  const { conversationId: activeConversationId } = useChatSessionContext()

  const conversations = useMemo<HistoryConversation[]>(() => {
    return localConversations.map((conv) => ({
      id: conv.id,
      lastMessagedAt: conv.lastMessagedAt,
      lastUserMessage: extractLastUserMessage(conv.messages),
    }))
  }, [localConversations])

  const groupedConversations = useMemo(
    () => groupConversations(conversations),
    [conversations],
  )

  return (
    <ConversationList
      groupedConversations={groupedConversations}
      activeConversationId={activeConversationId}
      onDelete={removeConversation}
    />
  )
}
