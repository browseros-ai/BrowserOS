import { keepPreviousData, useQueryClient } from '@tanstack/react-query'
import type { UIMessage } from 'ai'
import { Loader2 } from 'lucide-react'
import type { FC } from 'react'
import { useMemo } from 'react'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { useConversations } from '@/lib/conversations/conversationStorage'
import { GetProfileIdByUserIdDocument } from '@/lib/conversations/graphql/uploadConversationDocument'
import { getQueryKeyFromDocument } from '@/lib/graphql/getQueryKeyFromDocument'
import { useGraphqlInfiniteQuery } from '@/lib/graphql/useGraphqlInfiniteQuery'
import { useGraphqlMutation } from '@/lib/graphql/useGraphqlMutation'
import { useGraphqlQuery } from '@/lib/graphql/useGraphqlQuery'
import { useChatSessionContext } from '../layout/ChatSessionContext'
import { ConversationList } from './components/ConversationList'
import type { HistoryConversation } from './components/types'
import { extractLastUserMessage, groupConversations } from './components/utils'
import {
  DeleteConversationDocument,
  GetConversationsForHistoryDocument,
} from './graphql/chatHistoryDocument'
import { LocalChatHistory } from './local/LocalChatHistory'

const RemoteChatHistory: FC<{ userId: string }> = ({ userId }) => {
  const { conversationId: activeConversationId } = useChatSessionContext()
  const queryClient = useQueryClient()

  const { data: profileData } = useGraphqlQuery(GetProfileIdByUserIdDocument, {
    userId,
  })
  const profileId = profileData?.profileByUserId?.rowId

  const {
    data: graphqlData,
    isLoading: isLoadingConversations,
    isFetching,
    hasNextPage,
    isFetchingNextPage,
    fetchNextPage,
  } = useGraphqlInfiniteQuery(
    GetConversationsForHistoryDocument,
    // biome-ignore lint/style/noNonNullAssertion: guarded by enabled
    (cursor) => ({ profileId: profileId!, after: cursor }),
    {
      enabled: !!profileId,
      initialPageParam: undefined,
      getNextPageParam: (lastPage) =>
        lastPage.conversations?.pageInfo.hasNextPage
          ? lastPage.conversations.pageInfo.endCursor
          : undefined,
      placeholderData: keepPreviousData,
    },
  )

  const deleteConversationMutation = useGraphqlMutation(
    DeleteConversationDocument,
    {
      onSuccess: () => {
        queryClient.invalidateQueries({
          queryKey: [
            getQueryKeyFromDocument(GetConversationsForHistoryDocument),
          ],
        })
      },
    },
  )

  const handleDelete = (id: string) => {
    deleteConversationMutation.mutate({ rowId: id })
  }

  const conversations = useMemo<HistoryConversation[]>(() => {
    if (!graphqlData?.pages) return []

    return graphqlData.pages.flatMap((page) =>
      (page.conversations?.nodes ?? [])
        .filter((node): node is NonNullable<typeof node> => node !== null)
        .map((node) => {
          const messages = node.conversationMessages.nodes
            .filter((m): m is NonNullable<typeof m> => m !== null)
            .map((m) => m.message as UIMessage)

          const timestamp = node.lastMessagedAt.endsWith('Z')
            ? node.lastMessagedAt
            : `${node.lastMessagedAt}Z`

          return {
            id: node.rowId,
            lastMessagedAt: new Date(timestamp).getTime(),
            lastUserMessage: extractLastUserMessage(messages),
          }
        }),
    )
  }, [graphqlData])

  const groupedConversations = useMemo(
    () => groupConversations(conversations),
    [conversations],
  )

  if (!profileId || isLoadingConversations) {
    return (
      <div className="flex flex-1 items-center justify-center py-12">
        <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
      </div>
    )
  }

  return (
    <ConversationList
      groupedConversations={groupedConversations}
      activeConversationId={activeConversationId}
      onDelete={handleDelete}
      hasNextPage={hasNextPage}
      isFetchingNextPage={isFetchingNextPage}
      onLoadMore={fetchNextPage}
      isRefreshing={isFetching && !isLoadingConversations}
    />
  )
}

export const ChatHistory: FC = () => {
  const { sessionInfo } = useSessionInfo()
  const userId = sessionInfo.user?.id
  // needed to initiate remote-sync
  useConversations()

  if (userId) {
    return <RemoteChatHistory userId={userId} />
  }

  return <LocalChatHistory />
}
