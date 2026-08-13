import { useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useSessionInfo } from '@/lib/auth/sessionStorage'
import { conversationStorage } from '@/lib/conversations/conversationStorage'
import { uploadConversations } from '@/lib/conversations/uploadConversationsToGraphql'
import { sentry } from '@/lib/sentry/sentry'
import {
  importServerConversation,
  SERVER_CONVERSATIONS_QUERY_KEY,
} from './conversations.hooks'
import { migrateLegacyConversations } from './conversations-migration.helpers'

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
