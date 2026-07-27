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
} from './conversation-scope'
import { createConversationUploadScheduler } from './conversation-upload-scheduler'
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
    ;(async () => {
      if (await conversationsOwnerBackfilledStorage.getValue()) return
      const current = (await conversationStorage.getValue()) ?? []
      const result = backfillConversationOwners(current, effectiveOwnerId)
      if (cancelled) return
      if (result.changed)
        await conversationStorage.setValue(result.conversations)
      await conversationsOwnerBackfilledStorage.setValue(true)
    })().catch((error) => {
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
    scheduleConversationUpload(userId ? conversations : [], userId ?? null)
  }, [userId, conversations])

  useEffect(() => {
    conversationStorage.getValue().then(setAllConversations)
    const unwatch = conversationStorage.watch((newValue) => {
      setAllConversations(newValue ?? [])
    })
    return unwatch
  }, [])

  const removeConversation = async (id: string) => {
    const current = (await conversationStorage.getValue()) ?? []
    await conversationStorage.setValue(current.filter((c) => c.id !== id))
    await removeConversationExecutionHistory(id)
  }

  const saveConversation = async (id: string, messages: UIMessage[]) => {
    const current = (await conversationStorage.getValue()) ?? []
    const plan = planConversationSave(current, id, messages)
    if (!plan) return

    // Stamp the current owner on the saved conversation so it stays scoped to
    // whoever authored it (undefined when signed out).
    const owned = plan.conversations.map((conversation) =>
      conversation.id === id
        ? { ...conversation, owner: userId }
        : conversation,
    )

    await conversationStorage.setValue(owned)
    await Promise.all(
      plan.removedConversationIds.map(removeConversationExecutionHistory),
    )
  }

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
