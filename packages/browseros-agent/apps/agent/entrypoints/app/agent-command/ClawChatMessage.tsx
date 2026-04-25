import { CheckCircle2, Copy, Loader2, XCircle } from 'lucide-react'
import { type FC, useCallback } from 'react'
import {
  Message,
  MessageAction,
  MessageActions,
  MessageContent,
  MessageResponse,
  MessageToolbar,
} from '@/components/ai-elements/message'
import {
  Reasoning,
  ReasoningContent,
  ReasoningTrigger,
} from '@/components/ai-elements/reasoning'
import { cn } from '@/lib/utils'
import type { ClawChatMessage as ClawChatMessageType } from './claw-chat-types'

function formatTokenCount(tokens: number): string {
  if (tokens < 1000) return String(tokens)
  return `${(tokens / 1000).toFixed(1)}K`
}

function formatCost(usd: number): string {
  if (usd < 0.005) return `$${usd.toFixed(4)}`
  return `$${usd.toFixed(2)}`
}

interface ClawChatMessageProps {
  message: ClawChatMessageType
}

export const ClawChatMessage: FC<ClawChatMessageProps> = ({ message }) => {
  const messageText = message.parts
    .filter((p) => p.type === 'text')
    .map((p) => p.text)
    .join('\n')

  const handleCopy = useCallback(() => {
    if (messageText) navigator.clipboard.writeText(messageText)
  }, [messageText])

  const hasCostData =
    message.role === 'assistant' &&
    (message.costUsd || message.tokensIn || message.tokensOut)

  return (
    <Message
      from={message.role}
      className="max-w-full group-[.is-user]:max-w-[80%]"
    >
      <MessageContent className="max-w-full overflow-hidden group-[.is-assistant]:w-full group-[.is-user]:max-w-full">
        {message.parts.map((part, index) => {
          const key = `${message.id}-part-${index}`

          switch (part.type) {
            case 'text':
              return (
                <MessageResponse
                  key={key}
                  className={cn(
                    'max-w-full overflow-hidden break-words',
                    '[&_[data-streamdown="code-block"]]:!w-full [&_[data-streamdown="code-block"]]:!max-w-full [&_[data-streamdown="code-block"]]:overflow-x-auto',
                    '[&_[data-streamdown="table-wrapper"]]:!w-full [&_[data-streamdown="table-wrapper"]]:!max-w-full [&_[data-streamdown="table-wrapper"]]:overflow-x-auto',
                    '[&_table]:w-max [&_table]:min-w-full',
                  )}
                >
                  {part.text}
                </MessageResponse>
              )

            case 'reasoning':
              return (
                <Reasoning key={key} className="w-full" defaultOpen={false}>
                  <ReasoningTrigger />
                  <ReasoningContent>{part.text}</ReasoningContent>
                </Reasoning>
              )

            case 'tool-call':
              return (
                <div
                  key={key}
                  className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm"
                >
                  {part.status === 'running' || part.status === 'pending' ? (
                    <Loader2 className="size-3.5 animate-spin text-muted-foreground" />
                  ) : null}
                  {part.status === 'completed' ? (
                    <CheckCircle2 className="size-3.5 text-green-500" />
                  ) : null}
                  {part.status === 'failed' ? (
                    <XCircle className="size-3.5 text-destructive" />
                  ) : null}
                  <span className="font-mono text-xs">{part.name}</span>
                  {part.error ? (
                    <span className="ml-auto text-destructive text-xs">
                      {part.error}
                    </span>
                  ) : null}
                </div>
              )

            case 'meta':
              return (
                <div key={key} className="text-muted-foreground text-xs">
                  {part.label}: {part.value}
                </div>
              )

            default:
              return null
          }
        })}

        {messageText ? (
          <MessageToolbar className="opacity-0 transition-opacity group-hover:opacity-100">
            <MessageActions>
              <MessageAction tooltip="Copy" onClick={handleCopy}>
                <Copy className="size-3.5" />
              </MessageAction>
            </MessageActions>
            {hasCostData ? (
              <div className="flex items-center gap-1.5 text-[11px] text-muted-foreground/50 tabular-nums">
                {message.tokensIn != null && message.tokensOut != null ? (
                  <span>
                    {formatTokenCount(message.tokensIn)} →{' '}
                    {formatTokenCount(message.tokensOut)}
                  </span>
                ) : null}
                {message.costUsd ? (
                  <span>{formatCost(message.costUsd)}</span>
                ) : null}
              </div>
            ) : null}
          </MessageToolbar>
        ) : null}
      </MessageContent>
    </Message>
  )
}
