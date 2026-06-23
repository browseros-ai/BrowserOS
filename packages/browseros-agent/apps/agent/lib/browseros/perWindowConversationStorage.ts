import { type StorageItemKey, storage } from '@wxt-dev/storage'

/**
 * Stores each window's active conversation under its own key, so a window-scoped
 * panel resumes that window's conversation when it (re)mounts instead of
 * starting blank. Per-key (not a shared map) so concurrent windows can't clobber
 * each other's entry. Session-scoped: window ids are not stable across restarts.
 */
function windowConversationKey(windowId: number): StorageItemKey {
  return `session:browseros.side_panel.window_conversation.${windowId}`
}

export async function getWindowConversation(
  windowId: number,
): Promise<string | null> {
  return storage.getItem<string>(windowConversationKey(windowId))
}

export async function setWindowConversation(
  windowId: number,
  conversationId: string,
): Promise<void> {
  await storage.setItem(windowConversationKey(windowId), conversationId)
}
