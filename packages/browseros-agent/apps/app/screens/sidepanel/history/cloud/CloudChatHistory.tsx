import { keepPreviousData, useQueryClient } from '@tanstack/react-query'
import type { UIMessage } from 'ai'
import type { FC } from 'react'
import { useEffect, useMemo } from 'react'
import { GetProfileIdByUserIdDocument } from '@/lib/conversations/graphql/uploadConversationDocument'
import { getQueryKeyFromDocument } from '@/lib/graphql/getQueryKeyFromDocument'
import { useChatSessionContext } from '@/modules/chat/chat-session-context'
import { useGraphqlInfiniteQuery } from '@/modules/graphql/graphql-infinite-query.hooks'
import { useGraphqlMutation } from '@/modules/graphql/graphql-mutation.hooks'
import { useGraphqlQuery } from '@/modules/graphql/graphql-query.hooks'
import { ConversationList } from '../components/ConversationList'
import type { HistoryConversation } from '../components/types'
import { extractLastUserMessage, groupConversations } from '../components/utils'
import {
  DeleteConversationDocument,
  GetConversationsForHistoryDocument,
} from '../graphql/chatHistoryDocument'
import {
  excludeLocalConversations,
  hasAnyConversation,
  shouldAdvanceCloudPage,
} from '../history-union.helpers'

export interface CloudChatHistoryProps {
  userId: string
  /** Ids already on this machine, so the same chat is not listed twice. */
  localIds: ReadonlySet<string>
}

/**
 * Conversations that were synced to the account before sync was turned off.
 *
 * Read only and clearly separated rather than merged into the local list: it
 * is a legacy shelf that empties when the cloud is retired, and blending it
 * into the local history would hide that. Interleaving the two by date would
 * also mean paginating two sources against one scroll position, which this
 * deliberately avoids.
 */
export const CloudChatHistory: FC<CloudChatHistoryProps> = ({
  userId,
  localIds,
}) => {
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
    isFetchNextPageError,
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
    () =>
      groupConversations(excludeLocalConversations(conversations, localIds)),
    [conversations, localIds],
  )
  const hasVisibleConversations = hasAnyConversation(groupedConversations)

  // Pagination is normally driven by a sentinel inside the rendered list, so a
  // page that deduplicates away to nothing would stop it dead: the section
  // renders null, the sentinel never mounts, and cloud-only conversations on
  // later pages stay invisible. That is the ordinary case right after this
  // ships, because the most recent conversations are the ones that exist in
  // both stores and they sort onto the first page.
  //
  // Advancing here is the only way to reach past them; it cannot be lifted
  // into an event handler because there is no interaction to hang it on, and
  // it terminates when the pages run out or a page fails.
  const advance = shouldAdvanceCloudPage({
    hasVisibleConversations,
    hasNextPage: Boolean(hasNextPage),
    isFetchingNextPage,
    isLoading: isLoadingConversations,
    hasPageError: isFetchNextPageError,
  })
  useEffect(() => {
    if (advance) fetchNextPage()
  }, [advance, fetchNextPage])

  // Nothing to announce until there is something here. The loading case is
  // silent too: this section sits below the local list, so a spinner would
  // shift content the user is already reading.
  if (!profileId || isLoadingConversations) return null
  if (!hasVisibleConversations) return null

  return (
    <section className="mt-6 border-border border-t pt-4">
      <div className="px-1 pb-1">
        <h2 className="font-semibold text-muted-foreground text-xs uppercase tracking-wider">
          Saved to your account
        </h2>
        <p className="text-muted-foreground text-xs">
          From before cloud sync was turned off. Still readable here, and not
          stored on this device.
        </p>
      </div>
      <ConversationList
        groupedConversations={groupedConversations}
        activeConversationId={activeConversationId}
        onDelete={handleDelete}
        hasNextPage={hasNextPage}
        isFetchingNextPage={isFetchingNextPage}
        onLoadMore={fetchNextPage}
        isRefreshing={isFetching && !isLoadingConversations}
      />
    </section>
  )
}
