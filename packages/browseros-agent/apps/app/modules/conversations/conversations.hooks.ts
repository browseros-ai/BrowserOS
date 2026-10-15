import type { ConversationSummary } from '@browseros/server-api'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { UIMessage } from 'ai'
import { removeConversationExecutionHistory } from '@/lib/execution-history/storage'
import {
  getServerApi,
  nullOnNotFound,
  serverData,
} from '@/lib/server-api/client'
import { uiMessages } from '@/lib/server-api/ui-messages'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

export const SERVER_CONVERSATIONS_QUERY_KEY = 'server-conversations'

export type ServerConversationSummary = Pick<
  ConversationSummary,
  'id' | 'lastMessagedAt'
> & { lastUserMessage: string }

export interface ServerConversation {
  id: string
  messages: UIMessage[]
  targetType: 'browseros' | 'claude' | 'codex' | 'custom'
  agentId?: string
}

export async function fetchServerConversations(): Promise<
  ServerConversationSummary[]
> {
  const client = await getServerApi()
  const { conversations } = await serverData(
    client.http.GET('/conversations'),
    (error) => `Failed to load conversations (${error.status})`,
  )
  return conversations.map((conversation) => ({
    id: conversation.id,
    lastMessagedAt: conversation.lastMessagedAt,
    lastUserMessage: conversation.lastUserMessage ?? '',
  }))
}

export async function fetchServerConversation(
  conversationId: string,
): Promise<ServerConversation | null> {
  const client = await getServerApi()
  const data = await nullOnNotFound(
    serverData(
      client.http.GET('/conversations/{conversationId}', {
        params: { path: { conversationId } },
      }),
      (error) => `Failed to load conversation (${error.status})`,
    ),
  )
  if (!data) return null
  // Stored messages are intentionally opaque JSON in the HTTP contract. The
  // AI SDK owns their UI representation; no server implementation types leak in.
  const messages = uiMessages(data.conversation.messages)
  return {
    id: data.conversation.id,
    messages,
    targetType: data.conversation.targetType,
    agentId: data.conversation.agentId,
  }
}

/** Deletes only the server row (tolerating 404); leaves execution history. */
export async function deleteServerConversationRow(
  conversationId: string,
): Promise<void> {
  const client = await getServerApi()
  await nullOnNotFound(
    serverData(
      client.http.DELETE('/conversations/{conversationId}', {
        params: { path: { conversationId } },
      }),
      (error) => `Failed to delete conversation (${error.status})`,
    ),
  )
}

export async function deleteServerConversation(
  conversationId: string,
): Promise<void> {
  await deleteServerConversationRow(conversationId)
  await removeConversationExecutionHistory(conversationId)
}

export function useServerConversations(
  enabled = true,
  refetchInterval?: number,
) {
  const { baseUrl, isLoading } = useAgentServerUrl()
  return useQuery({
    queryKey: [SERVER_CONVERSATIONS_QUERY_KEY, baseUrl],
    queryFn: fetchServerConversations,
    // Even if discovery failed, let the query report/retry that failure instead
    // of leaving history in a permanently disabled "pending" state.
    enabled: !isLoading && enabled,
    refetchInterval,
  })
}

export function useDeleteServerConversation() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: deleteServerConversation,
    onSuccess: () =>
      queryClient.invalidateQueries({
        queryKey: [SERVER_CONVERSATIONS_QUERY_KEY],
      }),
  })
}
