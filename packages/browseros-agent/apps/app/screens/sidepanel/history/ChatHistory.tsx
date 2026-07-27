import { keepPreviousData, useQueryClient } from '@tanstack/react-query'
import type { UIMessage } from 'ai'
import { Loader2 } from 'lucide-react'
import type { FC } from 'react'
import { useMemo } from 'react'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { GetProfileIdByUserIdDocument } from '@/lib/conversations/graphql/uploadConversationDocument'
import { useConversations } from '@/lib/conversations/useConversations'
import { getQueryKeyFromDocument } from '@/lib/graphql/getQueryKeyFromDocument'
import { useChatSessionContext } from '@/modules/chat/chat-session-context'
import { useGraphqlInfiniteQuery } from '@/modules/graphql/graphql-infinite-query.hooks'
import { useGraphqlMutation } from '@/modules/graphql/graphql-mutation.hooks'
import { useGraphqlQuery } from '@/modules/graphql/graphql-query.hooks'
import { ConversationList } from './components/ConversationList'
import type { HistoryConversation } from './components/types'
import {
  extractLastUserMessage,
  groupConversations,
  mergeHistoryConversations,
} from './components/utils'
import {
  DeleteConversationDocument,
  GetConversationsForHistoryDocument,
} from './graphql/chatHistoryDocument'
import { LocalChatHistory } from './local/LocalChatHistory'

/**
 * History for a signed-in user. Local conversations are the durable source of
 * truth (issue #559) and are always shown; the cloud is merged in by id so
 * older, cloud-only conversations still appear. Deleting removes both copies.
 */
const MergedChatHistory: FC<{ userId: string }> = ({ userId }) => {
  const { conversationId: activeConversationId } = useChatSessionContext()
  const queryClient = useQueryClient()

  // Local (durable) history plus its delete + execution-history cascade. This
  // also drives cloud sync of local conversations via useConversations.
  const { conversations: localConversations, removeConversation } =
    useConversations()

  const { data: profileData } = useGraphqlQuery(GetProfileIdByUserIdDocument, {
    userId,
  })
  const profileId = profileData?.profileByUserId?.rowId

  const {
    data: graphqlData,
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

  const remoteConversations = useMemo<HistoryConversation[]>(() => {
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

  const localHistory = useMemo<HistoryConversation[]>(
    () =>
      localConversations.map((conv) => ({
        id: conv.id,
        lastMessagedAt: conv.lastMessagedAt,
        lastUserMessage: extractLastUserMessage(conv.messages),
      })),
    [localConversations],
  )

  const merged = useMemo<HistoryConversation[]>(
    () => mergeHistoryConversations(localHistory, remoteConversations),
    [localHistory, remoteConversations],
  )

  const remoteIds = useMemo(
    () => new Set(remoteConversations.map((c) => c.id)),
    [remoteConversations],
  )

  const groupedConversations = useMemo(
    () => groupConversations(merged),
    [merged],
  )

  const handleDelete = async (id: string) => {
    // Delete both copies so the conversation cannot reappear from the other
    // source on the next merge.
    await removeConversation(id)
    if (remoteIds.has(id)) {
      deleteConversationMutation.mutate({ rowId: id })
    }
  }

  // Only block on a spinner when there is nothing local to show yet and the
  // cloud has not returned its first page. Once local exists it renders
  // immediately and the cloud folds in, so history is never blank while
  // signed in (issue #559).
  if (merged.length === 0 && !graphqlData) {
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
      isRefreshing={isFetching}
    />
  )
}

export const ChatHistory: FC = () => {
  const { sessionInfo } = useSessionInfo()
  const userId = sessionInfo.user?.id

  if (userId) {
    return <MergedChatHistory userId={userId} />
  }

  // Signed out: local-only view (which also runs the no-op sync). Local is
  // retained across auth changes, so signing out no longer empties history.
  return <LocalChatHistory />
}
