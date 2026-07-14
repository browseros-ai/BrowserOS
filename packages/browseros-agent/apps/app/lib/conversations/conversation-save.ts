import type { UIMessage } from 'ai'
import type { Conversation } from './conversationStorage'

const MAX_CONVERSATIONS = 50
const MAX_COMPARISON_DEPTH = 5
const MAX_COMPARISON_VALUES = 128
const MAX_CONTAINER_ENTRIES = 64

interface ComparisonBudget {
  remaining: number
}

/** Builds the next bounded conversation list, or returns null when the append-only chat snapshot has not advanced. */
export function planConversationSave(
  current: Conversation[],
  id: string,
  messages: UIMessage[],
  lastMessagedAt = Date.now(),
): {
  conversations: Conversation[]
  removedConversationIds: string[]
} | null {
  const existingIndex = current.findIndex(
    (conversation) => conversation.id === id,
  )

  if (existingIndex >= 0) {
    const existing = current[existingIndex]
    if (!haveMessagesChanged(existing.messages, messages)) return null

    const conversations = [...current]
    conversations[existingIndex] = {
      ...existing,
      messages,
      lastMessagedAt: Math.max(lastMessagedAt, existing.lastMessagedAt + 1),
    }

    return { conversations, removedConversationIds: [] }
  }

  const nextConversations = [
    { id, messages, lastMessagedAt },
    ...current,
  ].slice(0, MAX_CONVERSATIONS)

  return {
    conversations: nextConversations,
    removedConversationIds: current
      .slice(MAX_CONVERSATIONS - 1)
      .map((conversation) => conversation.id),
  }
}

function haveMessagesChanged(previous: UIMessage[], next: UIMessage[]) {
  if (previous.length !== next.length) return true

  const previousMessage = previous.at(-1)
  const nextMessage = next.at(-1)
  if (!previousMessage || !nextMessage) return false

  if (
    previousMessage.id !== nextMessage.id ||
    previousMessage.role !== nextMessage.role ||
    previousMessage.parts.length !== nextMessage.parts.length
  ) {
    return true
  }

  return (
    !areTerminalValuesEqual(previousMessage.metadata, nextMessage.metadata) ||
    !areTerminalValuesEqual(
      previousMessage.parts.at(-1),
      nextMessage.parts.at(-1),
    )
  )
}

function areTerminalValuesEqual(previous: unknown, next: unknown) {
  return areBoundedValuesEqual(previous, next, 0, {
    remaining: MAX_COMPARISON_VALUES,
  })
}

function areBoundedValuesEqual(
  previous: unknown,
  next: unknown,
  depth: number,
  budget: ComparisonBudget,
): boolean {
  if (Object.is(previous, next)) return true
  if (
    typeof previous !== typeof next ||
    previous === null ||
    next === null ||
    typeof previous !== 'object'
  ) {
    return false
  }

  const previousIsArray = Array.isArray(previous)
  if (previousIsArray !== Array.isArray(next)) return false
  if (depth >= MAX_COMPARISON_DEPTH || budget.remaining === 0) {
    return (
      Object.prototype.toString.call(previous) ===
      Object.prototype.toString.call(next)
    )
  }
  budget.remaining -= 1

  if (previousIsArray) {
    const previousArray = previous as unknown[]
    const nextArray = next as unknown[]
    if (previousArray.length !== nextArray.length) return false

    const comparedEntries = Math.min(
      previousArray.length,
      MAX_CONTAINER_ENTRIES,
    )
    for (let offset = 0; offset < comparedEntries; offset += 1) {
      const index =
        comparedEntries === 1
          ? 0
          : Math.round(
              (offset * (previousArray.length - 1)) / (comparedEntries - 1),
            )
      if (
        !areBoundedValuesEqual(
          previousArray[index],
          nextArray[index],
          depth + 1,
          budget,
        )
      ) {
        return false
      }
    }
    return true
  }

  const previousEntries = boundedEntries(previous as Record<string, unknown>)
  const nextEntries = boundedEntries(next as Record<string, unknown>)
  if (previousEntries.length !== nextEntries.length) return false

  for (let index = 0; index < previousEntries.length; index += 1) {
    const [previousKey, previousValue] = previousEntries[index]
    const [nextKey, nextValue] = nextEntries[index]
    if (
      previousKey !== nextKey ||
      !areBoundedValuesEqual(previousValue, nextValue, depth + 1, budget)
    ) {
      return false
    }
  }
  return true
}

function boundedEntries(value: Record<string, unknown>) {
  const entries: [string, unknown][] = []
  for (const key in value) {
    if (!Object.hasOwn(value, key)) continue
    entries.push([key, value[key]])
    if (entries.length > MAX_CONTAINER_ENTRIES) break
  }
  return entries
}
