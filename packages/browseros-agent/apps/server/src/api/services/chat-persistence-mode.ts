/**
 * Whether a chat request's conversation should be written to the database.
 *
 * `persist` is the field; `historyMode` is the shape it replaced and is still
 * accepted, because the extension updates independently of the browser binary
 * and a shipped build may send either. Absent means persist, which is the
 * opposite of what `historyMode` defaulted to: its default of 'cloud' meant a
 * caller that omitted it silently got no history, which was safe only while a
 * cloud still held the conversation.
 */
export function shouldPersist(request: {
  persist?: boolean
  historyMode?: 'local' | 'cloud'
}): boolean {
  if (request.persist !== undefined) return request.persist
  if (request.historyMode !== undefined) return request.historyMode === 'local'
  return true
}
