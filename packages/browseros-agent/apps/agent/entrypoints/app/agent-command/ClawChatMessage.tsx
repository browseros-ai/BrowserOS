import { CheckCircle2, Copy, Loader2, Wrench, XCircle } from 'lucide-react'
import { type FC, useCallback, useMemo } from 'react'
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
import {
  Task,
  TaskContent,
  TaskItem,
  TaskTrigger,
} from '@/components/ai-elements/task'
import { cn } from '@/lib/utils'
import type {
  ClawChatMessagePart,
  ClawChatMessage as ClawChatMessageType,
} from './claw-chat-types'

function formatCost(usd: number): string {
  if (usd < 0.005) return `$${usd.toFixed(4)}`
  return `$${usd.toFixed(2)}`
}

type ToolCallPart = Extract<ClawChatMessagePart, { type: 'tool-call' }>

interface RenderEntry {
  kind: 'text' | 'reasoning' | 'meta' | 'task'
  partIndex: number
  part?: ClawChatMessagePart
  tools?: ToolCallPart[]
}

/**
 * Build a render plan that groups all tool-call parts into a single Task
 * collapsible at their first appearance position. Other parts render in place.
 */
function buildRenderEntries(parts: ClawChatMessagePart[]): RenderEntry[] {
  const entries: RenderEntry[] = []
  const tools: ToolCallPart[] = []
  let taskInserted = false

  parts.forEach((part, partIndex) => {
    if (part.type === 'tool-call') {
      tools.push(part)
      if (!taskInserted) {
        entries.push({ kind: 'task', partIndex, tools })
        taskInserted = true
      }
    } else if (part.type === 'text') {
      entries.push({ kind: 'text', partIndex, part })
    } else if (part.type === 'reasoning') {
      entries.push({ kind: 'reasoning', partIndex, part })
    } else if (part.type === 'meta') {
      entries.push({ kind: 'meta', partIndex, part })
    }
  })

  return entries
}

function ToolStatusIcon({ status }: { status: ToolCallPart['status'] }) {
  if (status === 'running' || status === 'pending') {
    return (
      <Loader2 className="size-3.5 shrink-0 animate-spin text-muted-foreground" />
    )
  }
  if (status === 'completed') {
    return <CheckCircle2 className="size-3.5 shrink-0 text-green-500" />
  }
  return <XCircle className="size-3.5 shrink-0 text-destructive" />
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

  const entries = useMemo(
    () => buildRenderEntries(message.parts),
    [message.parts],
  )

  return (
    <Message
      from={message.role}
      className="max-w-full group-[.is-user]:max-w-[80%]"
    >
      <MessageContent className="max-w-full overflow-hidden group-[.is-assistant]:w-full group-[.is-user]:max-w-full">
        {entries.map((entry) => {
          const key = `${message.id}-entry-${entry.partIndex}`

          if (entry.kind === 'text' && entry.part?.type === 'text') {
            return (
              <MessageResponse
                key={key}
                // Historical messages are finalized — render immediately.
                // Streamdown's default "streaming" mode uses an idle-callback
                // debounce (300ms / 500ms idle) that paints empty content
                // first, which made history flash blank tool collapsibles
                // before text on every load.
                mode="static"
                parseIncompleteMarkdown={false}
                className={cn(
                  'max-w-full overflow-hidden break-words',
                  '[&_[data-streamdown="code-block"]]:!w-full [&_[data-streamdown="code-block"]]:!max-w-full [&_[data-streamdown="code-block"]]:overflow-x-auto',
                  '[&_[data-streamdown="table-wrapper"]]:!w-full [&_[data-streamdown="table-wrapper"]]:!max-w-full [&_[data-streamdown="table-wrapper"]]:overflow-x-auto',
                  '[&_table]:w-max [&_table]:min-w-full',
                )}
              >
                {entry.part.text}
              </MessageResponse>
            )
          }

          if (entry.kind === 'reasoning' && entry.part?.type === 'reasoning') {
            return (
              <Reasoning
                key={key}
                className="w-full"
                defaultOpen={false}
                duration={entry.part.duration}
              >
                <ReasoningTrigger />
                <ReasoningContent>{entry.part.text}</ReasoningContent>
              </Reasoning>
            )
          }

          if (entry.kind === 'meta' && entry.part?.type === 'meta') {
            return (
              <div key={key} className="text-muted-foreground text-xs">
                {entry.part.label}: {entry.part.value}
              </div>
            )
          }

          if (entry.kind === 'task' && entry.tools) {
            const tools = entry.tools
            const errorCount = tools.filter((t) => t.status === 'failed').length
            const taskTitle = `Agent activity (${tools.length} ${tools.length === 1 ? 'action' : 'actions'}${errorCount > 0 ? `, ${errorCount} failed` : ''})`

            return (
              <Task key={key} defaultOpen={false}>
                <TaskTrigger title={taskTitle} TriggerIcon={Wrench} />
                <TaskContent>
                  {tools.map((tool, idx) => (
                    <TaskItem
                      // biome-ignore lint/suspicious/noArrayIndexKey: tool order is stable within a finalized historical message
                      key={`${tool.name}-${tool.status}-${idx}`}
                      className="flex items-center gap-2"
                    >
                      <ToolStatusIcon status={tool.status} />
                      <span className="text-foreground text-xs">
                        {tool.label}
                      </span>
                      {tool.subject ? (
                        <span className="ml-1.5 truncate text-muted-foreground/70 text-xs">
                          · {tool.subject}
                        </span>
                      ) : null}
                      {tool.error ? (
                        <span className="ml-2 truncate text-destructive text-xs">
                          {tool.error}
                        </span>
                      ) : null}
                      {tool.durationMs != null ? (
                        <span className="ml-auto text-muted-foreground/60 text-xs tabular-nums">
                          {(tool.durationMs / 1000).toFixed(1)}s
                        </span>
                      ) : null}
                    </TaskItem>
                  ))}
                </TaskContent>
              </Task>
            )
          }

          return null
        })}

        {message.role === 'assistant' && messageText ? (
          <MessageToolbar>
            <MessageActions>
              <MessageAction tooltip="Copy" onClick={handleCopy}>
                <Copy className="size-3.5" />
              </MessageAction>
            </MessageActions>
            {message.costUsd ? (
              <span className="text-[11px] text-muted-foreground/50 tabular-nums">
                {formatCost(message.costUsd)}
              </span>
            ) : null}
          </MessageToolbar>
        ) : null}
      </MessageContent>
    </Message>
  )
}
