import type { ConversationRoutes } from '@browseros/server'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { UIMessage } from 'ai'
import { hc } from 'hono/client'
import { removeConversationExecutionHistory } from '@/lib/execution-history/storage'
import { resolveAgentServerUrlWithRetry } from '@/modules/browseros/agent-server-url.helpers'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

export const SERVER_CONVERSATIONS_QUERY_KEY = 'server-conversations'

export interface ServerConversationSummary {
  id: string
  lastMessagedAt: number
  lastUserMessage: string
}

async function conversationsClient() {
  const baseUrl = await resolveAgentServerUrlWithRetry()
  return hc<ConversationRoutes>(`${baseUrl}/conversations`)
}

export async function fetchServerConversations(): Promise<
  ServerConversationSummary[]
> {
  const client = await conversationsClient()
  const response = await client.index.$get()
  if (!response.ok) {
    throw new Error(`Failed to load conversations (${response.status})`)
  }
  const { conversations } = await response.json()
  return conversations.map((conversation) => ({
    id: conversation.id,
    lastMessagedAt: conversation.lastMessagedAt,
    lastUserMessage: conversation.lastUserMessage ?? '',
  }))
}

export async function fetchServerConversation(
  conversationId: string,
): Promise<{ id: string; messages: UIMessage[] } | null> {
  const client = await conversationsClient()
  const response = await client[':conversationId'].$get({
    param: { conversationId },
  })
  if (response.status === 404) return null
  if (!response.ok) {
    throw new Error(`Failed to load conversation (${response.status})`)
  }
  const data = await response.json()
  if (!('conversation' in data)) {
    throw new Error('Failed to load conversation')
  }
  // hc applies a JSON transform to the response type, and UIMessage[]'s union is
  // too deep for the compiler to instantiate through it (TS2589). The runtime
  // shape is UIMessage[] exactly as the server stored it, so assert it here at
  // the JSON boundary.
  const messages = data.conversation.messages as UIMessage[]
  return { id: data.conversation.id, messages }
}

export async function importServerConversation(conversation: {
  id: string
  messages: UIMessage[]
  lastMessagedAt: number
}): Promise<void> {
  const client = await conversationsClient()
  const response = await client[':conversationId'].$put({
    param: { conversationId: conversation.id },
    json: {
      messages: conversation.messages,
      lastMessagedAt: conversation.lastMessagedAt,
    },
  })
  if (!response.ok) {
    throw new Error(`Failed to import conversation (${response.status})`)
  }
}

/** Deletes only the server row (tolerating 404); leaves execution history. */
export async function deleteServerConversationRow(
  conversationId: string,
): Promise<void> {
  const client = await conversationsClient()
  const response = await client[':conversationId'].$delete({
    param: { conversationId },
  })
  if (!response.ok && response.status !== 404) {
    throw new Error(`Failed to delete conversation (${response.status})`)
  }
}

export async function deleteServerConversation(
  conversationId: string,
): Promise<void> {
  await deleteServerConversationRow(conversationId)
  await removeConversationExecutionHistory(conversationId)
}

export function useServerConversations(enabled = true) {
  const { baseUrl, isLoading } = useAgentServerUrl()
  return useQuery({
    queryKey: [SERVER_CONVERSATIONS_QUERY_KEY, baseUrl],
    queryFn: fetchServerConversations,
    enabled: Boolean(baseUrl) && !isLoading && enabled,
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
