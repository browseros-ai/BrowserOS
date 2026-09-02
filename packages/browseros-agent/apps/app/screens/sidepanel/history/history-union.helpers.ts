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

/**
 * Whether the cloud section should pull the next page on its own.
 *
 * Pagination is normally driven by a sentinel inside the rendered list, which
 * never mounts while the section has nothing visible. A page whose entries are
 * all present locally deduplicates away to nothing, so without this the
 * section stalls on that page and never reaches the cloud-only conversations
 * behind it.
 *
 * A failed page has to stop it. `hasNextPage` is derived from the last
 * successful page, so a rejected fetch leaves it true while the in-flight flag
 * clears, returning every input to its pre-fetch value. Advancing again on
 * that state retries a failing request forever with no user interaction.
 */
export function shouldAdvanceCloudPage(state: {
  hasVisibleConversations: boolean
  hasNextPage: boolean
  isFetchingNextPage: boolean
  isLoading: boolean
  hasPageError: boolean
}): boolean {
  if (state.hasVisibleConversations) return false
  if (state.hasPageError) return false
  if (state.isLoading || state.isFetchingNextPage) return false
  return state.hasNextPage
}
