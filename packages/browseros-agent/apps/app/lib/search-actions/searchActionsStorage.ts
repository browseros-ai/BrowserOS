import { storage } from '@wxt-dev/storage'
import type { ChatAction } from '@/lib/chat-actions/types'

/**
 * @public
 */
export interface SearchActionStorage {
  query: string
  mode: 'chat' | 'agent'
  action?: ChatAction
}

/**
 * @public
 */
export const searchActionsStorage = storage.defineItem<SearchActionStorage>(
  'local:search-actions',
)
