import type { UIMessage } from 'ai'
import type { ServerConversation } from '../conversations/conversations.hooks'
import type { SidepanelChatTarget } from './sidepanel-chat-targets'

export interface RestoredServerConversation {
  id: string
  messages: UIMessage[]
  targetType?: ServerConversation['targetType']
  agentId?: string
}

interface RestoreServerConversationOptions {
  conversationId: string
  fetchConversation: (
    conversationId: string,
  ) => Promise<RestoredServerConversation | null>
  isCancelled: () => boolean
  onRestore: (conversation: RestoredServerConversation) => void | Promise<void>
  onMissing?: () => void
  onError: (error: unknown) => void
  onSettled: () => void
}

/**
 * Restore a conversation from the local server. Unlike the old
 * extension-storage read this hits the network, so it guards two failure modes:
 * a conversation switch mid-flight must not apply a stale response
 * (`isCancelled`), and any failure must still settle the UI (`finally`) so it
 * never strands in the restoring state with the query param unresolved.
 */
export async function restoreServerConversation({
  conversationId,
  fetchConversation,
  isCancelled,
  onRestore,
  onMissing,
  onError,
  onSettled,
}: RestoreServerConversationOptions): Promise<void> {
  try {
    const conversation = await fetchConversation(conversationId)
    if (isCancelled()) return
    if (conversation) await onRestore(conversation)
    else onMissing?.()
  } catch (error) {
    if (isCancelled()) return
    onError(error)
  } finally {
    if (!isCancelled()) onSettled()
  }
}

/** ACP history belongs to its exact agent. Native chats can use any LLM provider. */
export function resolveRestoredChatTarget(
  conversation: RestoredServerConversation,
  targets: SidepanelChatTarget[],
  selected: SidepanelChatTarget | undefined,
): SidepanelChatTarget | undefined {
  if (conversation.targetType && conversation.targetType !== 'browseros') {
    return targets.find(
      (target) =>
        target.kind === 'acp' &&
        target.agentId === conversation.agentId &&
        target.agentType === conversation.targetType,
    )
  }
  return selected?.kind === 'llm'
    ? selected
    : targets.find((target) => target.kind === 'llm')
}
