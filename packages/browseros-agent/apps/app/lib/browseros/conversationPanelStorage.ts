import type { ConversationPanel } from '@browseros/shared/schemas/conversation-panels'
import { storage } from '@wxt-dev/storage'

export type ConversationPanelViews = Record<string, ConversationPanel>

/**
 * Background-owned tab routing table consumed by thin side-panel views.
 * Session storage survives worker suspension but resets with the browser,
 * matching the lifetime of Chrome tab ids.
 */
export const conversationPanelViewsStorage =
  storage.defineItem<ConversationPanelViews>(
    'session:browseros.side_panel.conversation_views',
    { fallback: {} },
  )

export function conversationForTab(
  views: ConversationPanelViews,
  tabId: number | undefined,
): ConversationPanel | undefined {
  return tabId === undefined ? undefined : views[String(tabId)]
}
