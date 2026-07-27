import { type StorageItemKey, storage } from '@wxt-dev/storage'

/**
 * Stores each tab's active conversation under its own key, so a tab-scoped
 * panel resumes that tab's conversation when active instead of
 * sharing/clobbering others. Session-scoped: tab ids are not stable across restarts.
 */
function tabConversationKey(tabId: number): StorageItemKey {
  return `session:browseros.side_panel.tab_conversation.${tabId}`
}

export async function getTabConversation(
  tabId: number,
): Promise<string | null> {
  return storage.getItem<string>(tabConversationKey(tabId))
}

export async function setTabConversation(
  tabId: number,
  conversationId: string,
): Promise<void> {
  await storage.setItem(tabConversationKey(tabId), conversationId)
}
