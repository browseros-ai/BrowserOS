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

export interface PromoteServerConversationsOptions {
  userId: string
  collect: () => Promise<Conversation[]>
  upload: (conversations: Conversation[], userId: string) => Promise<string[]>
  drain: (id: string) => Promise<void>
}

export interface PromoteResult {
  uploadedIds: string[]
  allUploaded: boolean
}

/**
 * Promotes the local server's (logged-out) history to the cloud, then drains
 * only the conversations the cloud confirmed. Draining is what keeps a later
 * sign-in under a different account from re-uploading someone else's retained
 * history, and it leaves any failed conversation on the server for a retry.
 */
export async function promoteServerConversations({
  userId,
  collect,
  upload,
  drain,
}: PromoteServerConversationsOptions): Promise<PromoteResult> {
  const conversations = await collect()
  if (conversations.length === 0) return { uploadedIds: [], allUploaded: true }

  const uploadedIds = await upload(conversations, userId)
  await Promise.all(uploadedIds.map((id) => drain(id)))

  return {
    uploadedIds,
    allUploaded: uploadedIds.length === conversations.length,
  }
}

/**
 * Returns a runner that executes tasks one at a time. The promote must not
 * overlap across an account switch: a serialized second promotion waits for the
 * first to upload and drain, so it never re-uploads the same unowned server rows
 * into a different account.
 */
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
