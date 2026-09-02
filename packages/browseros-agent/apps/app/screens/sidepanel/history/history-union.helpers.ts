import type {
  GroupedConversations,
  HistoryConversation,
} from './components/types'

/**
 * Drops cloud conversations that already exist on this machine.
 *
 * The same conversation id is used by extension storage, the local server and
 * the cloud, so a conversation that was synced before sync was turned off
 * exists in both lists. Local wins: it is the copy that keeps working.
 */
export function excludeLocalConversations(
  cloud: readonly HistoryConversation[],
  localIds: ReadonlySet<string>,
): HistoryConversation[] {
  return cloud.filter((conversation) => !localIds.has(conversation.id))
}

/** Whether a grouped set has anything in it, in any bucket. */
export function hasAnyConversation(grouped: GroupedConversations): boolean {
  return (
    grouped.today.length > 0 ||
    grouped.thisWeek.length > 0 ||
    grouped.thisMonth.length > 0 ||
    grouped.older.length > 0
  )
}
