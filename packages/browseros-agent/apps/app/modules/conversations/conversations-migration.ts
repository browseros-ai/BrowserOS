import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { conversationStorage } from '@/lib/conversations/conversationStorage'
import { uploadConversations } from '@/lib/conversations/uploadConversationsToGraphql'
import { sentry } from '@/lib/sentry/sentry'
import {
  deleteServerConversationRow,
  fetchServerConversation,
  fetchServerConversations,
  importServerConversation,
  SERVER_CONVERSATIONS_QUERY_KEY,
} from './conversations.hooks'
import {
  collectServerConversations,
  createSerialRunner,
  migrateLegacyConversations,
  promoteServerConversations,
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

// Module-scoped so the promote survives history remounts (once per sign-in, not
// once per history open); reset when the user is absent, or when a promote does
// not fully complete, so leftovers retry.
let lastPromotedUserId: string | undefined
// Serialize so an account switch cannot run two promotions over the same
// undrained server rows concurrently (which could upload them into two accounts).
const runPromoteExclusive = createSerialRunner()

/**
 * On sign-in, promote server-held (logged-out) history to the cloud (draining
 * each conversation the cloud confirms, so it cannot leak to a later sign-in),
 * then run `onPromoted` (e.g. to refresh the cloud history list) when anything
 * landed.
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
    runPromoteExclusive(() =>
      promoteServerConversations({
        userId,
        collect: () =>
          collectServerConversations({
            listSummaries: fetchServerConversations,
            loadDetail: fetchServerConversation,
          }),
        upload: uploadConversations,
        drain: deleteServerConversationRow,
      }),
    )
      .then((result) => {
        // Reset the guard whenever the promote did not fully complete, even if
        // this effect was cancelled, so leftovers are retried and never linger
        // leak-eligible. Only the UI refresh is gated on cancellation.
        if (!result.allUploaded) lastPromotedUserId = undefined
        if (!cancelled && result.uploadedIds.length > 0) onPromoted?.()
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
