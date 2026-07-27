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
