import type { UIMessage } from 'ai'

export const CONTENT_FILTER_NOTICE =
  'The server rejected the content being sent or generated. Try rephrasing and send it again.'

function hasVisibleText(message: UIMessage): boolean {
  return message.parts.some(
    (part) => part.type === 'text' && part.text.trim().length > 0,
  )
}

function withNoticePart(message: UIMessage): UIMessage {
  return {
    ...message,
    parts: [...message.parts, { type: 'text', text: CONTENT_FILTER_NOTICE }],
  }
}

/** Adds a visible assistant notice when a provider filters an otherwise blank reply. */
export function addContentFilterNotice(
  messages: UIMessage[],
  responseMessage: UIMessage,
  finishReason?: string,
): UIMessage[] {
  if (finishReason !== 'content-filter' || hasVisibleText(responseMessage)) {
    return messages
  }

  const index = messages.findIndex(
    (message) => message.id === responseMessage.id,
  )
  const nextResponse = withNoticePart(responseMessage)

  if (index === -1) {
    return [...messages, nextResponse]
  }

  const next = [...messages]
  next[index] = nextResponse
  return next
}
