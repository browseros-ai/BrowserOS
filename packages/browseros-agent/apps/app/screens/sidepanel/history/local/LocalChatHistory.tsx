import type { FC } from 'react'
import { useMemo } from 'react'
import { useChatSessionContext } from '@/modules/chat/chat-session-context'
import {
  useDeleteServerConversation,
  useServerConversations,
} from '@/modules/conversations/conversations.hooks'
import { ConversationList } from '../components/ConversationList'
import type { HistoryConversation } from '../components/types'
import { groupConversations } from '../components/utils'

export const LocalChatHistory: FC = () => {
  const { data: serverConversations = [] } = useServerConversations()
  const deleteConversation = useDeleteServerConversation()
  const { conversationId: activeConversationId } = useChatSessionContext()

  const conversations = useMemo<HistoryConversation[]>(() => {
    return serverConversations.map((conversation) => ({
      id: conversation.id,
      lastMessagedAt: conversation.lastMessagedAt,
      lastUserMessage: conversation.lastUserMessage,
    }))
  }, [serverConversations])

  const groupedConversations = useMemo(
    () => groupConversations(conversations),
    [conversations],
  )

  return (
    <ConversationList
      groupedConversations={groupedConversations}
      activeConversationId={activeConversationId}
      onDelete={(id) => deleteConversation.mutate(id)}
    />
  )
}
