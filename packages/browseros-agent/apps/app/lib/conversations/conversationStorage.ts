import { storage } from '@wxt-dev/storage'
import type { UIMessage } from 'ai'

export interface Conversation {
  id: string
  messages: UIMessage[]
  lastMessagedAt: number
  /**
   * User id that authored the conversation, or undefined when authored
   * signed-out. Scopes local history to its owner so it survives that user's
   * sign-out without leaking to another account on the same profile (#559).
   */
  owner?: string
  /**
   * Set when the owner was inferred by the upgrade backfill rather than
   * observed at authoring time. Such records stay local-only and are never
   * uploaded, so a shared profile can't push one account's pre-upgrade history
   * into the first signer's cloud (#559). Sticky: continuing the conversation
   * does not clear it, so a later signer can't promote an adopted record.
   */
  localOnly?: boolean
}

export const conversationStorage = storage.defineItem<Conversation[]>(
  'local:conversations',
  {
    fallback: [],
  },
)

/**
 * Set once the pre-owner local history has been stamped with its owner on
 * upgrade, so the one-time backfill never runs again (#559).
 */
export const conversationsOwnerBackfilledStorage = storage.defineItem<boolean>(
  'local:conversationsOwnerBackfilled',
  {
    fallback: false,
  },
)
