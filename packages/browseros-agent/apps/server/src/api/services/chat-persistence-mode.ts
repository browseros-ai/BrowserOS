/**
 * Whether a chat request's conversation should be written to the database.
 *
 * `persist` is the field; `historyMode` is the shape it replaced and is still
 * accepted, because the extension updates independently of the browser binary
 * and a shipped build may send either. Absent means persist, which is the
 * opposite of what `historyMode` defaulted to: its default of 'cloud' meant a
 * caller that omitted it silently got no history, which was safe only while a
 * cloud still held the conversation.
 *
 * A scheduled run is not a chat and is decided here rather than by the field,
 * because the scheduled caller sends neither and a shipped extension cannot be
 * asked to start. Its record is the run history the scheduler keeps; writing it
 * to the conversations table would put every background run in the user's chat
 * list, which is the one thing flipping the default above would otherwise do.
 */
export function shouldPersist(request: {
  persist?: boolean
  historyMode?: 'local' | 'cloud'
  isScheduledTask?: boolean
}): boolean {
  if (request.isScheduledTask) return false
  if (request.persist !== undefined) return request.persist
  if (request.historyMode !== undefined) return request.historyMode === 'local'
  return true
}
