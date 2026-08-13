import type { UIMessage } from 'ai'
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

export interface CollectServerConversationsOptions {
  listSummaries: () => Promise<Array<{ id: string; lastMessagedAt: number }>>
  loadDetail: (
    id: string,
  ) => Promise<{ id: string; messages: UIMessage[] } | null>
}

/**
 * Reads every server conversation with its messages, shaped for a cloud upload.
 * Drops any conversation deleted between the list and its detail fetch.
 */
export async function collectServerConversations({
  listSummaries,
  loadDetail,
}: CollectServerConversationsOptions): Promise<Conversation[]> {
  const summaries = await listSummaries()
  const details = await Promise.all(
    summaries.map(async (summary) => {
      const detail = await loadDetail(summary.id)
      return detail
        ? {
            id: detail.id,
            messages: detail.messages,
            lastMessagedAt: summary.lastMessagedAt,
          }
        : null
    }),
  )
  return details.filter(
    (conversation): conversation is Conversation => conversation !== null,
  )
}
