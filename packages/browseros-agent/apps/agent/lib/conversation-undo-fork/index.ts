import type { UIMessage } from 'ai'

// ═══════════════════════════════════════════════════════════════════════════
// Index-based functions (for internal use & testing)
// ═══════════════════════════════════════════════════════════════════════════

/**
 * Find the index of the user message that starts the turn
 * containing the message at `messageIndex`.
 *
 * A "turn" = one user message + its immediate assistant response(s).
 * Walk backwards from messageIndex to find the user message.
 */
export function findTurnStartIndex(
  messages: UIMessage[],
  messageIndex: number,
): number {
  if (messageIndex < 0 || messageIndex >= messages.length) return -1
  for (let i = messageIndex; i >= 0; i--) {
    if (messages[i].role === 'user') return i
  }
  // Fallback: the message itself
  return messageIndex
}

/**
 * Find the index of the last message in the turn that starts at `turnStartIndex`.
 * This is either the last consecutive assistant message after the user message,
 * or the user message itself if there's no assistant response yet.
 */
export function findTurnEndIndex(
  messages: UIMessage[],
  turnStartIndex: number,
): number {
  if (turnStartIndex < 0 || turnStartIndex >= messages.length)
    return turnStartIndex
  let end = turnStartIndex
  for (let i = turnStartIndex + 1; i < messages.length; i++) {
    if (messages[i].role === 'assistant') {
      end = i
    } else {
      break
    }
  }
  return end
}

/**
 * Undo: truncate all messages from the turn containing `messageIndex`.
 * Returns the remaining prefix.
 */
export function undoFromMessage(
  messages: UIMessage[],
  messageIndex: number,
): UIMessage[] {
  const turnStart = findTurnStartIndex(messages, messageIndex)
  if (turnStart === -1) return messages
  return messages.slice(0, turnStart)
}

/**
 * Fork: copy all messages up to (but not including) the turn containing `messageIndex`.
 * The forked conversation gets the prefix so the user can re-prompt from there.
 *
 * Returns the forked messages array.
 */
export function forkFromMessage(
  messages: UIMessage[],
  messageIndex: number,
): UIMessage[] {
  const turnStart = findTurnStartIndex(messages, messageIndex)
  if (turnStart === -1) return []
  // Include everything before this turn — user can re-prompt
  return messages.slice(0, turnStart)
}

/**
 * Edit: replace the user message content at the turn containing `messageIndex`,
 * and truncate everything after it (so the LLM re-processes from that point).
 */
export function editMessageAtTurn(
  messages: UIMessage[],
  messageIndex: number,
  newContent: string,
): UIMessage[] {
  const turnStart = findTurnStartIndex(messages, messageIndex)
  if (turnStart === -1) return messages

  const prefix = messages.slice(0, turnStart)
  const originalMessage = messages[turnStart]

  const editedMessage: UIMessage = {
    ...originalMessage,
    parts: [{ type: 'text' as const, text: newContent }],
  }

  return [...prefix, editedMessage]
}

// ═══════════════════════════════════════════════════════════════════════════
// ID-based functions (for use from React components)
// ═══════════════════════════════════════════════════════════════════════════

function findIndexById(messages: UIMessage[], messageId: string): number {
  return messages.findIndex((m) => m.id === messageId)
}

/** Undo from the turn containing the message with the given ID */
export function undoFromMessageId(
  messages: UIMessage[],
  messageId: string,
): UIMessage[] {
  const idx = findIndexById(messages, messageId)
  if (idx === -1) return messages
  return undoFromMessage(messages, idx)
}

/** Fork from the turn containing the message with the given ID */
export function forkFromMessageId(
  messages: UIMessage[],
  messageId: string,
): { messages: UIMessage[] } | null {
  const idx = findIndexById(messages, messageId)
  if (idx === -1) return null
  const forked = forkFromMessage(messages, idx)
  return forked.length > 0 ? { messages: forked } : null
}

/** Edit the user message at the turn containing the given ID */
export function editMessageById(
  messages: UIMessage[],
  messageId: string,
  newContent: string,
): UIMessage[] | null {
  const idx = findIndexById(messages, messageId)
  if (idx === -1) return null
  return editMessageAtTurn(messages, idx, newContent)
}
