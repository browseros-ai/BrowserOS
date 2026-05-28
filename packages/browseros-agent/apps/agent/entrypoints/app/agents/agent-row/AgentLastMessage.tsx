import { Quote } from 'lucide-react'
import type { FC } from 'react'
import { firstNonBlankLine, truncate } from './agent-row.helpers'

interface AgentLastMessageProps {
  message: string | null
}

const PREVIEW_CHARS = 110

/**
 * Inline preview of the most recent user message. Renders as a quoted,
 * italic line so the row reads like a conversation snippet rather than
 * a label-and-value pair. No hover-card — opening the agent's chat is
 * the canonical way to read the full message.
 */
export const AgentLastMessage: FC<AgentLastMessageProps> = ({ message }) => {
  if (!message) {
    return (
      <p className="mt-1 text-muted-foreground/70 text-xs italic">
        No messages yet — start a chat
      </p>
    )
  }
  const preview = truncate(firstNonBlankLine(message), PREVIEW_CHARS)
  return (
    <p className="mt-1.5 flex min-w-0 items-start gap-1.5 text-foreground/85 text-sm italic leading-snug">
      <Quote
        className="mt-1 size-3 shrink-0 text-muted-foreground/60"
        aria-hidden
      />
      <span className="min-w-0 flex-1 truncate">{preview}</span>
    </p>
  )
}
