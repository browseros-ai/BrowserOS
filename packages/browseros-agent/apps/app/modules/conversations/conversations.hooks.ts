import { ServerApiClient, ServerApiError } from '@browseros/server-api/client'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { UIMessage } from 'ai'
import { removeConversationExecutionHistory } from '@/lib/execution-history/storage'
import { resolveAgentServerUrl } from '@/modules/browseros/agent-server-url.helpers'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

export const SERVER_CONVERSATIONS_QUERY_KEY = 'server-conversations'

export interface ServerConversationSummary {
  id: string
  lastMessagedAt: number
  lastUserMessage: string
}

export interface ServerConversation {
  id: string
  messages: UIMessage[]
  targetType: 'browseros' | 'claude' | 'codex' | 'custom'
  agentId?: string
}

async function conversationsClient() {
  const baseUrl = await resolveAgentServerUrl()
  return new ServerApiClient(baseUrl)
}

export async function fetchServerConversations(): Promise<
  ServerConversationSummary[]
> {
  const client = await conversationsClient()
  try {
    const { conversations } = await client.listConversations()
    return conversations.map((conversation) => ({
      id: conversation.id,
      lastMessagedAt: conversation.lastMessagedAt,
      lastUserMessage: conversation.lastUserMessage ?? '',
    }))
  } catch (error) {
    if (error instanceof ServerApiError) {
      throw new Error(`Failed to load conversations (${error.status})`, {
        cause: error,
      })
    }
    throw error
  }
}

export async function fetchServerConversation(
  conversationId: string,
): Promise<ServerConversation | null> {
  const client = await conversationsClient()
  try {
    const { conversation } = await client.getConversation({ conversationId })
    // The contract leaves AI SDK parts open so existing transcripts retain
    // all their fields when passed back to useChat.
    return {
      id: conversation.id,
      messages: conversation.messages as UIMessage[],
      targetType: conversation.targetType,
      agentId: conversation.agentId ?? undefined,
    }
  } catch (error) {
    if (error instanceof ServerApiError && error.status === 404) return null
    throw error
  }
}

/** Deletes only the server row (tolerating 404); leaves execution history. */
export async function deleteServerConversationRow(
  conversationId: string,
): Promise<void> {
  const client = await conversationsClient()
  try {
    await client.deleteConversation({ conversationId })
  } catch (error) {
    if (!(error instanceof ServerApiError && error.status === 404)) throw error
  }
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
