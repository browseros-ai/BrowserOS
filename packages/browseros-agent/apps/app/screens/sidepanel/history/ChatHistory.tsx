import { type FC, useMemo } from 'react'
import { useChatSessionContext } from '@/modules/chat/chat-session-context'
import {
  useDeleteServerConversation,
  useServerConversations,
} from '@/modules/conversations/conversations.hooks'
import { ConversationList } from './components/ConversationList'
import type { HistoryConversation } from './components/types'
import { groupConversations } from './components/utils'

/**
 * Conversations stored on this machine.
 *
 * This used to be the union of two lists, the local server's and the account's,
 * with the local one first. The account half is gone along with sign-in, so
 * there is one source again and no deduplication to do between them.
 */
export const ChatHistory: FC = () => {
  const { data: serverConversations = [] } = useServerConversations()
  const deleteConversation = useDeleteServerConversation()
  const { conversationId: activeConversationId } = useChatSessionContext()

  const conversations = useMemo<HistoryConversation[]>(
    () =>
      serverConversations.map((conversation) => ({
        id: conversation.id,
        lastMessagedAt: conversation.lastMessagedAt,
        lastUserMessage: conversation.lastUserMessage,
      })),
    [serverConversations],
  )

  const groupedConversations = useMemo(
    () => groupConversations(conversations),
    [conversations],
  )

  return (
    <main className="mt-4 flex h-full flex-1 flex-col overflow-y-auto">
      <ConversationList
        groupedConversations={groupedConversations}
        activeConversationId={activeConversationId}
        onDelete={(id) => deleteConversation.mutate(id)}
        emptyMessage="No conversations on this device yet"
      />
    </main>
  )
}
