import type { UIMessage } from 'ai'
import { useEffect, useMemo, useState } from 'react'
import { lastSignedInUserStorage } from '../auth/lastSignedInUser'
import { useSessionInfo } from '../auth/sessionStorage'
import { removeConversationExecutionHistory } from '../execution-history/storage'
import { sentry } from '../sentry/sentry'
import { planConversationSave } from './conversation-save'
import {
  backfillConversationOwners,
  filterConversationsByOwner,
  resolveEffectiveOwnerId,
  selectUploadableConversations,
  stampConversationOwner,
} from './conversation-scope'
import { createConversationUploadScheduler } from './conversation-upload-scheduler'
import { runExclusive } from './conversation-write-queue'
import {
  type Conversation,
  conversationStorage,
  conversationsOwnerBackfilledStorage,
} from './conversationStorage'
import { uploadConversationsToGraphql } from './uploadConversationsToGraphql'

const scheduleConversationUpload = createConversationUploadScheduler(
  uploadConversationsToGraphql,
  {
    onError: (error) => {
      sentry.captureException(error, {
        extra: { message: 'Failed to upload local conversations' },
      })
    },
  },
)

export function useConversations() {
  const [allConversations, setAllConversations] = useState<Conversation[]>()
  const [lastSignedInUserId, setLastSignedInUserId] = useState<string | null>(
    null,
  )

  const { sessionInfo } = useSessionInfo()
  const userId = sessionInfo.user?.id

  useEffect(() => {
    lastSignedInUserStorage.getValue().then(setLastSignedInUserId)
    const unwatch = lastSignedInUserStorage.watch((value) => {
      setLastSignedInUserId(value ?? null)
    })
    return unwatch
  }, [])

  // Remember the signed-in user so their history stays scoped to them after
  // sign-out (issue #559).
  useEffect(() => {
    if (userId) lastSignedInUserStorage.setValue(userId)
  }, [userId])

  const effectiveOwnerId = resolveEffectiveOwnerId(userId, lastSignedInUserId)

  // One-time upgrade migration: conversations created before owner tracking
  // have no owner and would otherwise be hidden by the owner filter. Stamp them
  // for the first effective owner we see, once, so a user's pre-existing history
  // stays with them (#559). Deferred until an owner exists, so a never-signed-in
  // install keeps its history anonymous until first sign-in.
  useEffect(() => {
    if (!effectiveOwnerId) return
    let cancelled = false
    runExclusive(async () => {
      if (cancelled) return
      if (await conversationsOwnerBackfilledStorage.getValue()) return
      const current = (await conversationStorage.getValue()) ?? []
      const result = backfillConversationOwners(current, effectiveOwnerId)
      if (cancelled) return
      if (result.changed)
        await conversationStorage.setValue(result.conversations)
      await conversationsOwnerBackfilledStorage.setValue(true)
    }).catch((error) => {
      sentry.captureException(error, {
        extra: { message: 'Failed to backfill conversation owners' },
      })
    })
    return () => {
      cancelled = true
    }
  }, [effectiveOwnerId])

  // Only surface the effective identity's conversations so another account
  // signing into the same browser profile never sees them (#559).
  const conversations = useMemo(
    () => filterConversationsByOwner(allConversations ?? [], effectiveOwnerId),
    [allConversations, effectiveOwnerId],
  )

  useEffect(() => {
    // An empty snapshot cancels work queued before logout or local deletion.
    // Backfill-adopted (localOnly) records are never uploaded so a shared
    // profile can't push one account's pre-upgrade history into another's cloud.
    scheduleConversationUpload(
      userId ? selectUploadableConversations(conversations) : [],
      userId ?? null,
    )
  }, [userId, conversations])

  useEffect(() => {
    conversationStorage.getValue().then(setAllConversations)
    const unwatch = conversationStorage.watch((newValue) => {
      setAllConversations(newValue ?? [])
    })
    return unwatch
  }, [])

  const removeConversation = (id: string) =>
    runExclusive(async () => {
      const current = (await conversationStorage.getValue()) ?? []
      await conversationStorage.setValue(current.filter((c) => c.id !== id))
      await removeConversationExecutionHistory(id)
    })

  const saveConversation = (id: string, messages: UIMessage[]) =>
    runExclusive(async () => {
      const current = (await conversationStorage.getValue()) ?? []
      const plan = planConversationSave(current, id, messages)
      if (!plan) return

      // Stamp the effective owner so a signed-out user continuing their own
      // (last signed-in) history keeps it scoped to them rather than dropping it
      // to an anonymous record that the owner filter would then hide (#559).
      const owned = stampConversationOwner(
        plan.conversations,
        id,
        effectiveOwnerId,
      )

      await conversationStorage.setValue(owned)
      await Promise.all(
        plan.removedConversationIds.map(removeConversationExecutionHistory),
      )
    })

  const getConversation = (id: string) => {
    return conversations.find((c) => c.id === id)
  }

  return {
    conversations,
    effectiveOwnerId,
    removeConversation,
    saveConversation,
    getConversation,
  }
}
