import {
  BotIcon,
  CheckCircle2,
  CircleDashed,
  Clock,
  Loader2,
  ShieldX,
  XCircle,
} from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
import {
  Task,
  TaskContent,
  TaskItem,
  TaskTrigger,
} from '@/components/ai-elements/task'
import {
  TOOL_NOT_RUN,
  TOOL_STOPPED,
} from '@/lib/messaging/server/tool-output-status'
import type {
  ToolInvocationInfo,
  ToolInvocationState,
} from './getMessageSegments'

export interface ToolBatchProps {
  tools: ToolInvocationInfo[]
  isLastBatch: boolean
  isLastMessage: boolean
  isStreaming: boolean
}

export const ToolBatch: FC<ToolBatchProps> = ({
  tools,
  isLastBatch,
  isLastMessage,
  isStreaming,
}) => {
  const shouldBeOpen = isLastMessage && isLastBatch && isStreaming
  const [isOpen, setIsOpen] = useState(shouldBeOpen)
  const [hasUserInteracted, setHasUserInteracted] = useState(false)

  useEffect(() => {
    if (isLastMessage && !hasUserInteracted) {
      if (isLastBatch) {
        setIsOpen(isStreaming)
      } else {
        setIsOpen(false)
      }
    }
  }, [isStreaming, isLastMessage, isLastBatch, hasUserInteracted])

  const completedCount = tools.filter((t) => isToolCompleted(t.state)).length
  const stoppedCount = tools.filter(
    (t) => t.skippedStatus === TOOL_STOPPED,
  ).length
  const notRunCount = tools.filter(
    (t) => t.skippedStatus === TOOL_NOT_RUN,
  ).length
  const actionCount = tools.length - stoppedCount - notRunCount
  const triggerTitle = [
    actionCount > 0 && `${completedCount}/${actionCount} actions completed`,
    stoppedCount > 0 &&
      formatSkippedCount(stoppedCount, TOOL_STOPPED, actionCount),
    notRunCount > 0 &&
      formatSkippedCount(notRunCount, TOOL_NOT_RUN, actionCount),
  ]
    .filter(Boolean)
    .join(' · ')

  const onManualToggle = (newState: boolean) => {
    setHasUserInteracted(true)
    setIsOpen(newState)
  }

  return (
    <Task open={isOpen} onOpenChange={onManualToggle}>
      <TaskTrigger title={triggerTitle} TriggerIcon={BotIcon} />
      <TaskContent>
        {tools.map((tool) => (
          <div key={tool.toolCallId}>
            <TaskItem className="flex items-center gap-2">
              <ToolStatusIcon
                state={tool.state}
                skippedStatus={tool.skippedStatus}
              />
              <span className="flex-1">{formatToolName(tool.toolName)}</span>
              {tool.skippedStatus && (
                <span aria-hidden="true">{tool.skippedStatus}</span>
              )}
            </TaskItem>
          </div>
        ))}
      </TaskContent>
    </Task>
  )
}

const formatSkippedCount = (
  count: number,
  status: NonNullable<ToolInvocationInfo['skippedStatus']>,
  actionCount: number,
) => {
  const action = count === 1 ? 'action' : 'actions'
  return actionCount > 0
    ? `${count} ${status.toLowerCase()}`
    : `${count} ${action} ${status.toLowerCase()}`
}

const formatToolName = (name: string) => {
  return name
    ?.replace(/_/g, ' ')
    ?.replace(/([a-z])([A-Z])/g, '$1 $2')
    ?.replace(/^./, (s) => s.toUpperCase())
}

const isToolCompleted = (state: ToolInvocationState) =>
  state === 'result' || state === 'output-available'

const isToolInProgress = (state: ToolInvocationState) =>
  state === 'call' || state === 'input-available'

const isToolError = (state: ToolInvocationState) => state === 'output-error'

const isToolDenied = (state: ToolInvocationState) => state === 'output-denied'

const isToolWaitingForApproval = (state: ToolInvocationState) =>
  state === 'approval-requested'

const ToolStatusIcon: FC<
  Pick<ToolInvocationInfo, 'state' | 'skippedStatus'>
> = ({ state, skippedStatus }) => {
  if (skippedStatus) {
    return (
      <CircleDashed
        aria-label={skippedStatus}
        role="img"
        className="h-3.5 w-3.5 text-muted-foreground"
      />
    )
  }
  if (isToolCompleted(state)) {
    return <CheckCircle2 className="h-3.5 w-3.5 text-green-500" />
  }
  if (isToolWaitingForApproval(state)) {
    return <Clock className="h-3.5 w-3.5 text-yellow-500" />
  }
  if (isToolDenied(state)) {
    return <ShieldX className="h-3.5 w-3.5 text-red-400" />
  }
  if (isToolInProgress(state)) {
    return (
      <Loader2 className="h-3.5 w-3.5 animate-spin text-[var(--accent-orange)]" />
    )
  }
  if (isToolError(state)) {
    return <XCircle className="h-3.5 w-3.5 text-destructive" />
  }
  return <CircleDashed className="h-3.5 w-3.5 text-muted-foreground" />
}
