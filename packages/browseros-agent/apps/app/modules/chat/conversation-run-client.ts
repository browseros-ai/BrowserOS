import type { ConversationRunSnapshot } from '@browseros/server-api'
import { ServerApiClient } from '@browseros/server-api-client'
import type { UIMessage } from 'ai'
import { serverData } from '@/lib/server-api/client'

export type ConversationRunState = Omit<
  ConversationRunSnapshot,
  'messages' | 'chunkCount'
> & { messages: UIMessage[]; chunkCount?: number }

export async function fetchConversationRunState(
  serverUrl: string,
  conversationId: string,
  fetchImpl: typeof fetch = fetch,
  signal?: AbortSignal,
): Promise<ConversationRunState> {
  const client = new ServerApiClient(serverUrl, { fetch: fetchImpl })
  const value: unknown = await serverData(
    client.http.GET('/chat/{conversationId}/state', {
      params: { path: { conversationId } },
      cache: 'no-store',
      signal,
    }),
    (error) => `Failed to load active conversation (${error.status})`,
  )
  if (!isConversationRunState(value)) {
    throw new Error('Invalid active conversation state')
  }
  return value
}

export function conversationReconnectUrl(
  serverUrl: string,
  conversationId: string,
  runId?: string,
): string {
  const url = `${serverUrl}/chat/${encodeURIComponent(conversationId)}/stream`
  return runId ? `${url}?runId=${encodeURIComponent(runId)}` : url
}

function isConversationRunState(value: unknown): value is ConversationRunState {
  if (!value || typeof value !== 'object') return false
  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.conversationId === 'string' &&
    typeof candidate.runId === 'string' &&
    ['running', 'completed', 'aborted', 'failed'].includes(
      String(candidate.status),
    ) &&
    Array.isArray(candidate.messages)
  )
}
