import type { UIMessage } from 'ai'

export interface RestoredServerConversation {
  id: string
  messages: UIMessage[]
}

interface RestoreServerConversationOptions {
  conversationId: string
  fetchConversation: (
    conversationId: string,
  ) => Promise<RestoredServerConversation | null>
  isCancelled: () => boolean
  onRestore: (conversation: RestoredServerConversation) => void
  onError: (error: unknown) => void
  onSettled: () => void
}

/**
 * Restore a logged-out conversation from the local server. Unlike the old
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
  onError,
  onSettled,
}: RestoreServerConversationOptions): Promise<void> {
  try {
    const conversation = await fetchConversation(conversationId)
    if (isCancelled()) return
    if (conversation) onRestore(conversation)
  } catch (error) {
    if (isCancelled()) return
    onError(error)
  } finally {
    if (!isCancelled()) onSettled()
  }
}
