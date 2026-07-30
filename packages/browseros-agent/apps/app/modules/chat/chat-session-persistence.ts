import type { ChatStatus, UIMessage } from 'ai'
import { stripImageToolOutputs } from './tool-output-strip'

export function didStreamingTurnFinish(
  previousStatus: ChatStatus,
  status: ChatStatus,
) {
  const wasStreaming =
    previousStatus === 'streaming' || previousStatus === 'submitted'
  return wasStreaming && (status === 'ready' || status === 'error')
}

export function getPersistableMessages(messages: UIMessage[]) {
  return stripImageToolOutputs(
    messages.filter((message) => message.parts?.length > 0),
  )
}

/**
 * Whether a chat's history may be written to durable storage or the cloud.
 * False in incognito so nothing is saved (#1189); the `isTemporaryChat` seam
 * lets a future "temporary chat" toggle opt out the same way.
 */
export function shouldPersistHistory(
  isIncognito: boolean,
  isTemporaryChat = false,
): boolean {
  return !isIncognito && !isTemporaryChat
}
