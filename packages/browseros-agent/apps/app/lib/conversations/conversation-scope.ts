import type { Conversation } from './conversationStorage'

/**
 * The identity whose local history is shown: the current user, or the last
 * signed-in user when signed out (issue #559). undefined means never signed in
 * (an anonymous local session). Keeping the last signed-in user lets a user who
 * just signed out still see their own history, while a different account that
 * signs into the same browser profile never sees it.
 */
export function resolveEffectiveOwnerId(
  userId: string | undefined,
  lastSignedInUserId: string | null,
): string | undefined {
  return userId ?? lastSignedInUserId ?? undefined
}

/** Keeps only the conversations owned by the effective identity. */
export function filterConversationsByOwner(
  conversations: Conversation[],
  effectiveOwnerId: string | undefined,
): Conversation[] {
  return conversations.filter(
    (conversation) => (conversation.owner ?? undefined) === effectiveOwnerId,
  )
}

/**
 * Stamps the given owner onto every conversation that predates owner tracking
 * (no owner yet), leaving already-owned ones untouched. Run once on upgrade so
 * a user's pre-existing local history stays scoped to them instead of vanishing
 * behind the owner filter (#559). Returns `changed: false` when nothing needed
 * stamping so callers can skip the write.
 */
export function backfillConversationOwners(
  conversations: Conversation[],
  ownerId: string,
): { changed: boolean; conversations: Conversation[] } {
  let changed = false
  const next = conversations.map((conversation) => {
    if (conversation.owner != null) return conversation
    changed = true
    return { ...conversation, owner: ownerId }
  })
  return changed ? { changed, conversations: next } : { changed, conversations }
}
