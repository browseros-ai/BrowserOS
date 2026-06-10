import { storage } from '@wxt-dev/storage'
import type { UIMessage } from 'ai'

export interface Conversation {
  id: string
  messages: UIMessage[]
  lastMessagedAt: number
}

export const conversationStorage = storage.defineItem<Conversation[]>(
  'local:conversations',
  {
    fallback: [],
  },
)
