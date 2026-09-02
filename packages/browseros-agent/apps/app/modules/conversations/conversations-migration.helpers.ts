import type { Conversation } from '@/lib/conversations/conversationStorage'

export interface MigrateLegacyConversationsOptions {
  conversations: Conversation[]
  importToServer: (conversation: Conversation) => Promise<{ imported: boolean }>
  /** Reads back a server row, to check a skipped import already holds it all. */
  loadFromServer: (
    id: string,
  ) => Promise<{ messages: Array<{ id: string }> } | null>
}

/** Whether every message in the legacy copy is already on the server row. */
function serverHoldsEveryMessage(
  legacy: Conversation,
  server: { messages: Array<{ id: string }> } | null,
): boolean {
  if (!server) return false
  const stored = new Set(server.messages.map((message) => message.id))
  return legacy.messages.every((message) => stored.has(message.id))
}

/**
 * One-shot migration of pre-upgrade `local:conversations` into the local
 * server. Returns the ids that were handled so the caller can drain them from
 * storage; anything not returned is left in place for the next attempt.
 *
 * This used to send a logged-in user's history to the cloud instead. It now
 * takes the local path for everyone, which is the direction the rest of this
 * work moves data.
 *
 * The import is insert-if-absent, so a conversation whose id is already on the
 * server is answered with a success that wrote nothing. Reporting that as
 * handled would delete the legacy copy against a server row that might be an
 * older, shorter version of the same conversation, losing whatever it does not
 * contain. A skipped import is therefore only handled once the server row is
 * confirmed to hold every message the legacy copy has.
 */
export async function migrateLegacyConversations({
  conversations,
  importToServer,
  loadFromServer,
}: MigrateLegacyConversationsOptions): Promise<string[]> {
  if (conversations.length === 0) return []

  const migrated: string[] = []
  for (const conversation of conversations) {
    try {
      const { imported } = await importToServer(conversation)
      if (imported) {
        migrated.push(conversation.id)
        continue
      }

      const server = await loadFromServer(conversation.id)
      if (serverHoldsEveryMessage(conversation, server)) {
        migrated.push(conversation.id)
      }
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
