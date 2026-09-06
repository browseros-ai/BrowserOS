import type { ConversationPanelAssignment } from '@browseros/shared/schemas/conversation-panels'
import { storage } from '@wxt-dev/storage'

/** Successful browser effects survive MV3 suspension independently of run status. */
export type ConversationPanelView = Pick<
  ConversationPanelAssignment,
  'tabId' | 'conversationId'
> &
  Partial<Pick<ConversationPanelAssignment, 'runId' | 'status'>> & {
    openedRunId?: string
    /** A manual draft/history selection wins over an old server binding. */
    manual?: boolean
  }
export type ConversationPanelViews = Record<string, ConversationPanelView>

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
): ConversationPanelView | undefined {
  return tabId === undefined ? undefined : views[String(tabId)]
}
