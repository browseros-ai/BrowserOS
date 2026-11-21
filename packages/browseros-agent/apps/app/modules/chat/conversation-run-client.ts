import {
  ServerApiClient,
  type ServerApiClientOptions,
  ServerApiError,
} from '@browseros/server-api/client'
import type { components } from '@browseros/server-api/openapi'
import type { UIMessage } from 'ai'

export type ConversationRunState = Pick<
  components['schemas']['RunState'],
  'conversationId' | 'runId' | 'status'
> & { messages: UIMessage[] }

/**
 * Loads the server's transcript before the panel follows any running turn's stream, or
 * `undefined` when the server knows no run for the conversation (`404`).
 */
export async function fetchConversationRunState(
  serverUrl: string,
  conversationId: string,
  signal?: AbortSignal,
  fetchImpl?: ServerApiClientOptions['fetch'],
): Promise<ConversationRunState | undefined> {
  let state: unknown
  try {
    state = await new ServerApiClient(serverUrl, {
      fetch: fetchImpl,
    }).getRunState({ conversationId }, signal)
  } catch (error) {
    if (error instanceof ServerApiError && error.status === 404)
      return undefined
    throw error
  }
  if (!isConversationRunState(state)) {
    throw new Error('Invalid active conversation state')
  }
  return state
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
  return (
    'conversationId' in value &&
    typeof value.conversationId === 'string' &&
    'runId' in value &&
    typeof value.runId === 'string' &&
    'status' in value &&
    ['running', 'completed', 'aborted', 'failed'].includes(
      String(value.status),
    ) &&
    'messages' in value &&
    Array.isArray(value.messages)
  )
}
