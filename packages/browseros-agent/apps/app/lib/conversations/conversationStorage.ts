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
}

export const conversationStorage = storage.defineItem<Conversation[]>(
  'local:conversations',
  {
    fallback: [],
  },
)
