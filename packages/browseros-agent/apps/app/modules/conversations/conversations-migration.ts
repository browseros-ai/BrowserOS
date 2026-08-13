import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { conversationStorage } from '@/lib/conversations/conversationStorage'
import { uploadConversations } from '@/lib/conversations/uploadConversationsToGraphql'
import { sentry } from '@/lib/sentry/sentry'
import {
  fetchServerConversation,
  fetchServerConversations,
  importServerConversation,
  SERVER_CONVERSATIONS_QUERY_KEY,
} from './conversations.hooks'
import {
  collectServerConversations,
  migrateLegacyConversations,
} from './conversations-migration.helpers'

/**
 * Drains any pre-upgrade `local:conversations` to their new home (cloud when
 * logged in, the local server otherwise). Idempotent: once storage is drained
 * subsequent runs are no-ops.
 */
export function useLegacyConversationMigration(): void {
  const { sessionInfo } = useSessionInfo()
  const userId = sessionInfo.user?.id
  const queryClient = useQueryClient()

  useEffect(() => {
    let cancelled = false
    const run = async () => {
      const conversations = (await conversationStorage.getValue()) ?? []
      if (cancelled || conversations.length === 0) return

      const handledIds = await migrateLegacyConversations({
        conversations,
        isLoggedIn: !!userId,
        userId,
        importToServer: importServerConversation,
        uploadToCloud: uploadConversations,
      })
      if (cancelled || handledIds.length === 0) return

      const current = (await conversationStorage.getValue()) ?? []
      await conversationStorage.setValue(
        current.filter((conversation) => !handledIds.includes(conversation.id)),
      )
      if (!userId) {
        queryClient.invalidateQueries({
          queryKey: [SERVER_CONVERSATIONS_QUERY_KEY],
        })
      }
    }
    run().catch((error) => {
      sentry.captureException(error, {
        extra: { message: 'Legacy conversation migration failed' },
      })
    })
    return () => {
      cancelled = true
    }
  }, [userId, queryClient])
}

/**
 * Uploads the local server's (logged-out) conversations to the cloud so they
 * follow the user into their account. The server copy is kept (Decision 4);
 * `uploadConversations` is idempotent, so re-running only syncs the delta.
 * Returns how many conversations were considered.
 */
export async function promoteServerConversationsToCloud(
  userId: string,
): Promise<number> {
  const conversations = await collectServerConversations({
    listSummaries: fetchServerConversations,
    loadDetail: fetchServerConversation,
  })
  if (conversations.length === 0) return 0
  await uploadConversations(conversations, userId)
  return conversations.length
}

// Module-scoped so the promote survives history remounts (once per sign-in, not
// once per history open); reset when the user is absent so a re-sign-in
// promotes again.
let lastPromotedUserId: string | undefined

/**
 * On sign-in, promote server-held (logged-out) history to the cloud, then run
 * `onPromoted` (e.g. to refresh the cloud history list) when anything landed.
 */
export function useSignInConversationPromote(onPromoted?: () => void): void {
  const { sessionInfo } = useSessionInfo()
  const userId = sessionInfo.user?.id

  useEffect(() => {
    if (!userId) {
      lastPromotedUserId = undefined
      return
    }
    if (lastPromotedUserId === userId) return
    lastPromotedUserId = userId

    let cancelled = false
    promoteServerConversationsToCloud(userId)
      .then((count) => {
        if (!cancelled && count > 0) onPromoted?.()
      })
      .catch((error) => {
        lastPromotedUserId = undefined
        sentry.captureException(error, {
          extra: { message: 'Sign-in conversation promote failed' },
        })
      })
    return () => {
      cancelled = true
    }
  }, [userId, onPromoted])
}
