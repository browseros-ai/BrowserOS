/** Validates the persisted message envelope before it enters the AI SDK reducer. */
import type { UIMessage } from 'ai'

export function isUiMessage(value: unknown): value is UIMessage {
  if (!value || typeof value !== 'object') return false
  return (
    'id' in value &&
    typeof value.id === 'string' &&
    'role' in value &&
    (value.role === 'user' ||
      value.role === 'assistant' ||
      value.role === 'system') &&
    'parts' in value &&
    Array.isArray(value.parts)
  )
}

export function uiMessages(value: unknown): UIMessage[] {
  if (!Array.isArray(value) || !value.every(isUiMessage)) {
    throw new Error('Invalid conversation messages')
  }
  return value
}
