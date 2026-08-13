import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { UIMessage } from 'ai'
import { removeConversationExecutionHistory } from '@/lib/execution-history/storage'
import { resolveAgentServerUrlWithRetry } from '@/modules/browseros/agent-server-url.helpers'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

export const SERVER_CONVERSATIONS_QUERY_KEY = 'server-conversations'

export interface ServerConversationSummary {
  id: string
  lastMessagedAt: number
  lastUserMessage: string
}

interface ConversationListResponse {
  conversations: Array<{
    id: string
    lastMessagedAt: number
    lastUserMessage?: string
  }>
}

interface ConversationDetailResponse {
  conversation: { id: string; messages: UIMessage[] }
}

async function conversationsUrl(path: string): Promise<string> {
  const baseUrl = await resolveAgentServerUrlWithRetry()
  return `${baseUrl}/conversations${path}`
}

export async function fetchServerConversations(): Promise<
  ServerConversationSummary[]
> {
  const response = await fetch(await conversationsUrl(''))
  if (!response.ok) {
    throw new Error(`Failed to load conversations (${response.status})`)
  }
  const { conversations } = (await response.json()) as ConversationListResponse
  return conversations.map((conversation) => ({
    id: conversation.id,
    lastMessagedAt: conversation.lastMessagedAt,
    lastUserMessage: conversation.lastUserMessage ?? '',
  }))
}

export async function fetchServerConversation(
  conversationId: string,
): Promise<{ id: string; messages: UIMessage[] } | null> {
  const response = await fetch(
    await conversationsUrl(`/${encodeURIComponent(conversationId)}`),
  )
  if (response.status === 404) return null
  if (!response.ok) {
    throw new Error(`Failed to load conversation (${response.status})`)
  }
  const { conversation } = (await response.json()) as ConversationDetailResponse
  return { id: conversation.id, messages: conversation.messages }
}

export async function importServerConversation(conversation: {
  id: string
  messages: UIMessage[]
  lastMessagedAt: number
}): Promise<void> {
  const response = await fetch(
    await conversationsUrl(`/${encodeURIComponent(conversation.id)}`),
    {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        messages: conversation.messages,
        lastMessagedAt: conversation.lastMessagedAt,
      }),
    },
  )
  if (!response.ok) {
    throw new Error(`Failed to import conversation (${response.status})`)
  }
}

/** Deletes only the server row (tolerating 404); leaves execution history. */
export async function deleteServerConversationRow(
  conversationId: string,
): Promise<void> {
  const response = await fetch(
    await conversationsUrl(`/${encodeURIComponent(conversationId)}`),
    { method: 'DELETE' },
  )
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
