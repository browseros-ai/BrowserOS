import type { UIMessage } from 'ai'

export interface ConversationRunState {
  conversationId: string
  runId: string
  status: 'running' | 'completed' | 'aborted' | 'failed'
  messages: UIMessage[]
  replayMessages?: UIMessage[]
}

export async function fetchConversationRunState(
  serverUrl: string,
  conversationId: string,
  fetchImpl: typeof fetch = fetch,
  options?: { runId: string; signal?: AbortSignal },
): Promise<ConversationRunState> {
  const response = await fetchImpl(
    `${serverUrl}/chat/${encodeURIComponent(conversationId)}/state${options ? `?runId=${encodeURIComponent(options.runId)}` : ''}`,
    { cache: 'no-store', ...(options?.signal && { signal: options.signal }) },
  )
  if (!response.ok) {
    throw new Error(`Failed to load active conversation (${response.status})`)
  }
  const value: unknown = await response.json()
  if (!isConversationRunState(value)) {
    throw new Error('Invalid active conversation state')
  }
  if (
    value.conversationId !== conversationId ||
    (options && value.runId !== options.runId)
  ) {
    throw new Error('Conversation run changed')
  }
  return value
}

export function conversationReconnectUrl(
  serverUrl: string,
  conversationId: string,
  runId?: string,
): string {
  return `${serverUrl}/chat/${encodeURIComponent(conversationId)}/stream${runId ? `?runId=${encodeURIComponent(runId)}` : ''}`
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
