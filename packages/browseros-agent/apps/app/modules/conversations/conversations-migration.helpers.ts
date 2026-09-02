import type { Conversation } from '@/lib/conversations/conversationStorage'

export interface MigrateLegacyConversationsOptions {
  conversations: Conversation[]
  importToServer: (conversation: Conversation) => Promise<void>
}

/**
 * One-shot migration of pre-upgrade `local:conversations` into the local
 * server. Returns the ids that were handled so the caller can drain them from
 * storage; a conversation that fails to migrate is left for a retry.
 *
 * This used to send a logged-in user's history to the cloud instead. It now
 * takes the local path for everyone, which is the direction the rest of this
 * work moves data.
 */
export async function migrateLegacyConversations({
  conversations,
  importToServer,
}: MigrateLegacyConversationsOptions): Promise<string[]> {
  if (conversations.length === 0) return []

  const migrated: string[] = []
  for (const conversation of conversations) {
    try {
      await importToServer(conversation)
      migrated.push(conversation.id)
    } catch {
      // Leave unmigrated conversations in place for the next attempt.
    }
  }
  return migrated
}

export function createSerialRunner(): <T>(
  task: () => Promise<T>,
) => Promise<T> {
  let chain: Promise<unknown> = Promise.resolve()
  return <T>(task: () => Promise<T>): Promise<T> => {
    const result = chain.then(task, task)
    chain = result.then(
      () => undefined,
      () => undefined,
    )
    return result
  }
}
