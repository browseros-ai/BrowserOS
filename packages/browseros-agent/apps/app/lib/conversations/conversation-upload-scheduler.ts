import type { Conversation } from './conversationStorage'

const DEFAULT_UPLOAD_DEBOUNCE_MS = 1000

interface ConversationUploadSnapshot {
  conversations: Conversation[]
  generation: number
  revision: string
}

/** Creates a newest-wins debounce that never runs more than one conversation upload at a time. */
export function createConversationUploadScheduler(
  upload: (conversations: Conversation[]) => Promise<void>,
  options: {
    delayMs?: number
    onError?: (error: unknown) => void
  } = {},
) {
  const { delayMs = DEFAULT_UPLOAD_DEBOUNCE_MS, onError = () => undefined } =
    options
  let timeout: ReturnType<typeof setTimeout> | undefined
  let pendingSnapshot: ConversationUploadSnapshot | undefined
  let activeSnapshot: ConversationUploadSnapshot | undefined
  let uploadedRevision: string | undefined
  let activeScope: string | null | undefined
  let generation = 0

  function armTimeout() {
    if (timeout !== undefined || activeSnapshot || !pendingSnapshot) return
    timeout = setTimeout(() => {
      timeout = undefined
      void flush()
    }, delayMs)
  }

  async function flush() {
    if (activeSnapshot || !pendingSnapshot) return
    const snapshot = pendingSnapshot
    pendingSnapshot = undefined
    activeSnapshot = snapshot

    try {
      await upload(snapshot.conversations)
      if (snapshot.generation === generation) {
        uploadedRevision = snapshot.revision
      }
    } catch (error) {
      onError(error)
    } finally {
      activeSnapshot = undefined
      armTimeout()
    }
  }

  return (
    conversations: Conversation[] | undefined,
    scope: string | null = null,
  ) => {
    if (scope !== activeScope) {
      activeScope = scope
      generation += 1
      if (timeout !== undefined) clearTimeout(timeout)
      timeout = undefined
      pendingSnapshot = undefined
      uploadedRevision = undefined
    }
    if (conversations === undefined) return
    if (conversations.length === 0) {
      if (timeout !== undefined) clearTimeout(timeout)
      timeout = undefined
      pendingSnapshot = undefined
      return
    }

    const revision = conversationSnapshotRevision(conversations)
    if (
      pendingSnapshot?.revision === revision ||
      (activeSnapshot?.generation === generation &&
        activeSnapshot.revision === revision) ||
      uploadedRevision === revision
    ) {
      return
    }

    if (timeout !== undefined) clearTimeout(timeout)
    timeout = undefined
    pendingSnapshot = { conversations, generation, revision }
    armTimeout()
  }
}

function conversationSnapshotRevision(conversations: Conversation[]) {
  let revision = String(conversations.length)
  for (const conversation of conversations) {
    revision += `:${conversation.id.length}:${conversation.id}:${conversation.lastMessagedAt}`
  }
  return revision
}
