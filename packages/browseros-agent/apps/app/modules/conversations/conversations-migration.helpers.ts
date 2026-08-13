import type { Conversation } from '@/lib/conversations/conversationStorage'

export interface MigrateLegacyConversationsOptions {
  conversations: Conversation[]
  isLoggedIn: boolean
  userId: string | undefined
  importToServer: (conversation: Conversation) => Promise<void>
  uploadToCloud: (
    conversations: Conversation[],
    userId: string,
  ) => Promise<string[]>
}

/**
 * One-shot migration of pre-upgrade `local:conversations`. A logged-in user
 * keeps the old promote-to-cloud behavior; a logged-out user's history moves to
 * the local server. Returns the ids that were handled so the caller can drain
 * them from storage; a conversation that fails to migrate is left for a retry.
 */
export async function migrateLegacyConversations({
  conversations,
  isLoggedIn,
  userId,
  importToServer,
  uploadToCloud,
}: MigrateLegacyConversationsOptions): Promise<string[]> {
  if (conversations.length === 0) return []

  if (isLoggedIn) {
    return userId ? uploadToCloud(conversations, userId) : []
  }

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
