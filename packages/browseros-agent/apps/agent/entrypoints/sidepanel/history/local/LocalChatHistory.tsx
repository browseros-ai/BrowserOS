import type { FC } from 'react'
import { useMemo } from 'react'
import { toast } from 'sonner'
import { useConversations } from '@/lib/conversations/conversationStorage'
import { useChatSessionContext } from '../../layout/ChatSessionContext'
import { ConversationList } from '../components/ConversationList'
import type { HistoryConversation } from '../components/types'
import { extractLastUserMessage, groupConversations } from '../components/utils'

export const LocalChatHistory: FC = () => {
  const {
    conversations: localConversations,
    removeConversation,
    clearConversations,
  } = useConversations()
  const { conversationId: activeConversationId, resetConversation } =
    useChatSessionContext()

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

  const handleClearAll = async () => {
    try {
      await clearConversations()
      resetConversation()
      toast.success('Chat sessions cleared')
    } catch {
      toast.error('Failed to clear chat sessions')
    }
  }

  return (
    <ConversationList
      groupedConversations={groupedConversations}
      activeConversationId={activeConversationId}
      onDelete={removeConversation}
      onClearAll={handleClearAll}
    />
  )
}
