import type { UIMessage } from 'ai'

const MAX_MESSAGES = 10
const MAX_MESSAGE_CHARS = 65536

export interface ConversationMessage {
  role: 'user' | 'assistant'
  content: string
}

export function formatConversationHistory(
  messages: UIMessage[],
  userMessageId: string | undefined,
): ConversationMessage[] | undefined {
  // Rust appends the current user message itself, so replay ends before its ID.
  const currentIndex = messages.findIndex(
    (msg) => msg.role === 'user' && msg.id === userMessageId,
  )
  if (currentIndex <= 0) return undefined

  const recentMessages = messages.slice(
    Math.max(0, currentIndex - MAX_MESSAGES),
    currentIndex,
  )

  const history = recentMessages
    .map((msg) => {
      if (!msg.parts?.length) return null
      const role: 'user' | 'assistant' =
        msg.role === 'user' ? 'user' : 'assistant'
      const textContent = msg.parts
        .filter((part) => part.type === 'text')
        .map((part) => part.text)
        .join('\n')

      if (!textContent.trim()) return null

      const content =
        textContent.length > MAX_MESSAGE_CHARS
          ? `${textContent.slice(0, MAX_MESSAGE_CHARS)}... [truncated]`
          : textContent

      return { role, content }
    })
    .filter((msg): msg is ConversationMessage => msg !== null)

  return history.length ? history : undefined
}
