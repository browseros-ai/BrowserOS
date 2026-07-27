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
 * Chooses the copy to display when resuming a conversation: prefer the cloud
 * copy unless the local copy has more messages (never regress to fewer).
 * Returns undefined when neither source has the conversation.
 */
export function pickRicherMessages(
  local: UIMessage[] | undefined,
  remote: UIMessage[] | undefined,
): UIMessage[] | undefined {
  if (remote && (!local || remote.length >= local.length)) return remote
  return local ?? remote
}
