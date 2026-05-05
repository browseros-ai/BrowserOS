import { Bot, CheckCircle2, Loader2, Wrench, XCircle } from 'lucide-react'
import { type FC, useMemo } from 'react'
import {
  Message,
  MessageAttachment,
  MessageAttachments,
  MessageContent,
  MessageResponse,
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
import type {
  AgentConversationTurn,
  ToolEntry,
} from '@/lib/agent-conversations/types'
import { FileCardStrip } from './agent-conversation.file-card-strip'

interface ConversationMessageProps {
  turn: AgentConversationTurn
  streaming: boolean
  /**
   * Forwarded to the inline file-card strip's "View" / "+N"
   * button. Wired up by AgentCommandConversation so the strip can
   * deep-link straight into the Outputs rail at the matching turn
   * group. `null` here disables the strip's deep-link affordance
   * — the cards still open the preview Sheet directly.
   */
  onOpenOutputsRail?: ((turnId?: string | null) => void) | null
  /**
   * Render only the trailing FileCardStrip for this turn — used
   * when the turn's user / assistant text is already rendered
   * elsewhere (e.g. by `ClawChatMessage` from persisted history)
   * but the produced-files affordance would otherwise be lost.
   */
  stripOnly?: boolean
}

interface RenderEntry {
  kind: 'thinking' | 'text' | 'task'
  partIndex: number
  text?: string
  done?: boolean
  tools?: ToolEntry[]
}

/**
 * Build the render plan for an assistant turn:
 * - thinking and text parts render in place
 * - all tool-batch parts collapse into a single Task entry at their first
 *   appearance position, with tools listed in arrival order
 */
function buildRenderEntries(turn: AgentConversationTurn): RenderEntry[] {
  const entries: RenderEntry[] = []
  const aggregatedTools: ToolEntry[] = []
  let taskInserted = false

  turn.parts.forEach((part, partIndex) => {
    if (part.kind === 'thinking') {
      entries.push({
        kind: 'thinking',
        partIndex,
        text: part.text,
        done: part.done,
      })
    } else if (part.kind === 'text') {
      entries.push({ kind: 'text', partIndex, text: part.text })
    } else if (part.kind === 'tool-batch') {
      aggregatedTools.push(...part.tools)
      if (!taskInserted) {
        entries.push({
          kind: 'task',
          partIndex,
          tools: aggregatedTools,
        })
        taskInserted = true
      }
    }
  })

  return entries
}

function ToolStatusIcon({ status }: { status: ToolEntry['status'] }) {
  if (status === 'running') {
    return (
      <Loader2 className="size-3.5 shrink-0 animate-spin text-muted-foreground" />
    )
  }
  if (status === 'completed') {
    return <CheckCircle2 className="size-3.5 shrink-0 text-green-500" />
  }
  return <XCircle className="size-3.5 shrink-0 text-destructive" />
}

export const ConversationMessage: FC<ConversationMessageProps> = ({
  turn,
  streaming,
  onOpenOutputsRail,
  stripOnly,
}) => {
  const entries = useMemo(() => buildRenderEntries(turn), [turn])

  if (stripOnly) {
    if (!turn.producedFiles || turn.producedFiles.length === 0) return null
    return (
      <FileCardStrip
        turnId={turn.turnId ?? null}
        files={turn.producedFiles}
        onOpenRail={onOpenOutputsRail ?? (() => {})}
      />
    )
  }

  return (
    <div className="space-y-3">
      <Message from="user">
        <MessageContent>
          {turn.userAttachments && turn.userAttachments.length > 0 && (
            <MessageAttachments>
              {turn.userAttachments.map((attachment) => (
                <MessageAttachment
                  key={attachment.id}
                  data={{
                    type: 'file',
                    url: attachment.dataUrl ?? '',
                    mediaType: attachment.mediaType,
                    filename: attachment.name,
                  }}
                />
              ))}
            </MessageAttachments>
          )}
          {turn.userText && (
            <pre className="whitespace-pre-wrap font-sans text-sm">
              {turn.userText}
            </pre>
          )}
        </MessageContent>
      </Message>

      {entries.length > 0 && (
        <Message from="assistant">
          <MessageContent>
            {entries.map((entry) => {
              const key = `${turn.id}-entry-${entry.partIndex}`

              if (entry.kind === 'thinking') {
                return (
                  <Reasoning
                    key={key}
                    className="w-full"
                    isStreaming={!entry.done}
                    defaultOpen={!entry.done}
                  >
                    <ReasoningTrigger />
                    <ReasoningContent>{entry.text ?? ''}</ReasoningContent>
                  </Reasoning>
                )
              }

              if (entry.kind === 'text') {
                return (
                  <MessageResponse key={key}>
                    {entry.text ?? ''}
                  </MessageResponse>
                )
              }

              const tools = entry.tools ?? []
              const allDone = tools.every((t) => t.status !== 'running')
              const taskTitle = allDone
                ? `Agent activity (${tools.length} ${tools.length === 1 ? 'action' : 'actions'})`
                : `Working… (${tools.length} ${tools.length === 1 ? 'action' : 'actions'})`

              return (
                <Task key={key} defaultOpen={!turn.done}>
                  <TaskTrigger title={taskTitle} TriggerIcon={Wrench} />
                  <TaskContent>
                    {tools.map((tool) => (
                      <TaskItem
                        key={tool.id}
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
                        {tool.durationMs != null && (
                          <span className="ml-auto text-muted-foreground/60 text-xs tabular-nums">
                            {(tool.durationMs / 1000).toFixed(1)}s
                          </span>
                        )}
                      </TaskItem>
                    ))}
                  </TaskContent>
                </Task>
              )
            })}
          </MessageContent>
        </Message>
      )}

      {turn.producedFiles && turn.producedFiles.length > 0 ? (
        <FileCardStrip
          turnId={turn.turnId ?? null}
          files={turn.producedFiles}
          onOpenRail={onOpenOutputsRail ?? (() => {})}
        />
      ) : null}

      {!turn.done && turn.parts.length === 0 && streaming && (
        <div className="flex gap-2">
          <div className="flex size-7 shrink-0 items-center justify-center rounded-full bg-[var(--accent-orange)] text-white">
            <Bot className="size-3.5" />
          </div>
          <div className="flex items-center gap-1 rounded-xl rounded-tl-none border border-border/50 bg-card px-3 py-2.5 shadow-sm">
            <span className="size-1.5 animate-bounce rounded-full bg-[var(--accent-orange)] [animation-delay:-0.3s]" />
            <span className="size-1.5 animate-bounce rounded-full bg-[var(--accent-orange)] [animation-delay:-0.15s]" />
            <span className="size-1.5 animate-bounce rounded-full bg-[var(--accent-orange)]" />
          </div>
        </div>
      )}
    </div>
  )
}
