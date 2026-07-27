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
 * Returns a copy of `conversations` with `ownerId` stamped on the conversation
 * matching `id`, leaving the rest untouched. Callers pass the effective owner
 * (current user, else last signed-in) so a signed-out user continuing their own
 * history keeps it scoped to them instead of dropping it to an anonymous,
 * unowned record that then leaks into anonymous history (#559).
 */
export function stampConversationOwner(
  conversations: Conversation[],
  id: string,
  ownerId: string | undefined,
): Conversation[] {
  return conversations.map((conversation) =>
    conversation.id === id ? { ...conversation, owner: ownerId } : conversation,
  )
}

/**
 * Stamps the given owner onto every conversation that predates owner tracking
 * (no owner yet), leaving already-owned ones untouched. Run once on upgrade so
 * a user's pre-existing local history stays scoped to them instead of vanishing
 * behind the owner filter (#559). The inferred owner is marked `localOnly` so
 * the record is never uploaded: on a shared profile the first signer sees the
 * adopted history locally but it can't reach their cloud. Returns
 * `changed: false` when nothing needed stamping so callers can skip the write.
 */
export function backfillConversationOwners(
  conversations: Conversation[],
  ownerId: string,
): { changed: boolean; conversations: Conversation[] } {
  let changed = false
  const next = conversations.map((conversation) => {
    if (conversation.owner != null) return conversation
    changed = true
    return { ...conversation, owner: ownerId, localOnly: true }
  })
  return changed ? { changed, conversations: next } : { changed, conversations }
}

/**
 * The conversations that may be pushed to the cloud: everything except records
 * whose owner was only inferred by the upgrade backfill (`localOnly`). Keeps one
 * account's adopted pre-upgrade history out of another account's cloud (#559).
 */
export function selectUploadableConversations(
  conversations: Conversation[],
): Conversation[] {
  return conversations.filter((conversation) => !conversation.localOnly)
}
