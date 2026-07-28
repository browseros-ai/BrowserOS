import type { UIMessage } from 'ai'

/**
 * A signed-in conversation held locally while it is in flight, so it still
 * reaches the cloud (and therefore history) if the user navigates away before
 * the turn's cloud write lands (#559). `userId` scopes the entry to its author
 * so a different account signing into the same profile never flushes it.
 */
export interface ActiveConversationBufferEntry {
  id: string
  messages: UIMessage[]
  lastMessagedAt: number
  userId: string
}

/** Replaces the entry with the same id, else appends. One entry per conversation. */
export function upsertBufferEntry(
  entries: ActiveConversationBufferEntry[],
  entry: ActiveConversationBufferEntry,
): ActiveConversationBufferEntry[] {
  return [...entries.filter((e) => e.id !== entry.id), entry]
}

/** The entries authored by `userId` (the only ones this session may flush). */
export function selectBufferEntriesForUser(
  entries: ActiveConversationBufferEntry[],
  userId: string,
): ActiveConversationBufferEntry[] {
  return entries.filter((e) => e.userId === userId)
}

/** Drops the entries with the given ids. */
export function removeBufferEntries(
  entries: ActiveConversationBufferEntry[],
  ids: Iterable<string>,
): ActiveConversationBufferEntry[] {
  const idSet = new Set(ids)
  return entries.filter((e) => !idSet.has(e.id))
}
