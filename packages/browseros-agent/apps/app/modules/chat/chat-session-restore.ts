import type { UIMessage } from 'ai'
import type { ServerConversation } from '../conversations/conversations.hooks'
import type { ConversationRunState } from './conversation-run-client'
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
  /** The server's run for the conversation, `undefined` when it knows none. */
  fetchRunState: (
    conversationId: string,
  ) => Promise<ConversationRunState | undefined>
  isCancelled: () => boolean
  onRestore: (
    conversation: RestoredServerConversation,
    run: ConversationRunState | undefined,
  ) => void | Promise<void>
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
 *
 * The stored row is read before the run. The server checkpoints a running turn
 * into the row, so a row alone would freeze a live conversation half-answered;
 * the run read second still answers if the turn ended in between (a finished
 * run serves its whole transcript), and a deleted conversation is never read
 * past its missing row.
 */
export async function restoreServerConversation({
  conversationId,
  fetchConversation,
  fetchRunState,
  isCancelled,
  onRestore,
  onMissing,
  onError,
  onSettled,
}: RestoreServerConversationOptions): Promise<void> {
  try {
    const conversation = await fetchConversation(conversationId)
    if (isCancelled()) return
    if (!conversation) {
      onMissing?.()
      return
    }
    const run = await fetchRunState(conversationId)
    if (isCancelled()) return
    await onRestore(conversation, run)
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
