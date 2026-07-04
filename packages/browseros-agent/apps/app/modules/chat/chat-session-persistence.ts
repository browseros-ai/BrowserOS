import type { ChatStatus, UIMessage } from 'ai'

export function didStreamingTurnFinish(
  previousStatus: ChatStatus,
  status: ChatStatus,
) {
  const wasStreaming =
    previousStatus === 'streaming' || previousStatus === 'submitted'
  return wasStreaming && (status === 'ready' || status === 'error')
}

export function getPersistableMessages(messages: UIMessage[]) {
  return messages.filter((message) => message.parts?.length > 0)
}
