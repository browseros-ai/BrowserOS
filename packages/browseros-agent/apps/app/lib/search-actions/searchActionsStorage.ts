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

/** Commands are addressed to native hosts; hidden panels must not submit them. */
export const panelSearchCommandsStorage = storage.defineItem<
  Record<string, SearchActionStorage>
>('session:panel-search-commands', { fallback: {} })

// The background is the only command writer/consumer. Serialize read-modify-write
// so mounting twice or acknowledging a command cannot erase a newer delivery.
let pending: Promise<unknown> = Promise.resolve()
export function enqueuePanelSearch(
  tabId: number,
  action: SearchActionStorage,
): Promise<void> {
  const next = pending
    .catch(() => undefined)
    .then(async () => {
      const commands = await panelSearchCommandsStorage.getValue()
      await panelSearchCommandsStorage.setValue({
        ...commands,
        [String(tabId)]: action,
      })
    })
  pending = next
  return next
}

export function takePanelSearch(
  tabId: number,
): Promise<SearchActionStorage | null> {
  const next = pending
    .catch(() => undefined)
    .then(async () => {
      const commands = await panelSearchCommandsStorage.getValue()
      const action = commands[String(tabId)]
      if (!action) return null
      delete commands[String(tabId)]
      await panelSearchCommandsStorage.setValue(commands)
      return action
    })
  pending = next
  return next
}
