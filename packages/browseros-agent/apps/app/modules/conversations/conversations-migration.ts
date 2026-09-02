import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { conversationStorage } from '@/lib/conversations/conversationStorage'
import { sentry } from '@/lib/sentry/sentry'
import {
  fetchServerConversation,
  importServerConversation,
  SERVER_CONVERSATIONS_QUERY_KEY,
} from './conversations.hooks'
import {
  createSerialRunner,
  migrateLegacyConversations,
} from './conversations-migration.helpers'

/**
 * Drains any pre-upgrade `local:conversations` to their new home (cloud when
 * logged in, the local server otherwise). Idempotent: once storage is drained
 * subsequent runs are no-ops.
 */
export function useLegacyConversationMigration(): void {
  const { sessionInfo } = useSessionInfo()
  const _userId = sessionInfo.user?.id
  const queryClient = useQueryClient()

  useEffect(() => {
    let cancelled = false
    const run = async () => {
      const conversations = (await conversationStorage.getValue()) ?? []
      if (cancelled || conversations.length === 0) return

      const handledIds = await migrateLegacyConversations({
        conversations,
        importToServer: importServerConversation,
        loadFromServer: fetchServerConversation,
      })
      if (cancelled || handledIds.length === 0) return

      const current = (await conversationStorage.getValue()) ?? []
      await conversationStorage.setValue(
        current.filter((conversation) => !handledIds.includes(conversation.id)),
      )
      queryClient.invalidateQueries({
        queryKey: [SERVER_CONVERSATIONS_QUERY_KEY],
      })
    }
    run().catch((error) => {
      sentry.captureException(error, {
        extra: { message: 'Legacy conversation migration failed' },
      })
    })
    return () => {
      cancelled = true
    }
  }, [queryClient])
}

// Module-scoped so the promote survives history remounts (once per sign-in, not
// once per history open); reset when the user is absent, or when a promote does
// not fully complete, so leftovers retry.
let _lastPromotedUserId: string | undefined
// Serialize so an account switch cannot run two promotions over the same
// undrained server rows concurrently (which could upload them into two accounts).
const _runPromoteExclusive = createSerialRunner()

/**
 * On sign-in, promote server-held (logged-out) history to the cloud (draining
 * each conversation the cloud confirms, so it cannot leak to a later sign-in),
 * then run `onPromoted` (e.g. to refresh the cloud history list) when anything
 * landed.
 */
