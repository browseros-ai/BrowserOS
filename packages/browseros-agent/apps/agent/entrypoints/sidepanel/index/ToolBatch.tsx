import {
  BotIcon,
  CheckCircle2,
  CircleDashed,
  Clock,
  Loader2,
  ShieldCheck,
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
import { Button } from '@/components/ui/button'
import type {
  ToolInvocationInfo,
  ToolInvocationState,
} from './getMessageSegments'

interface ToolBatchProps {
  tools: ToolInvocationInfo[]
  isLastBatch: boolean
  isLastMessage: boolean
  isStreaming: boolean
  onApprove?: (approvalId: string) => void
  onDeny?: (approvalId: string) => void
}

export const ToolBatch: FC<ToolBatchProps> = ({
  tools,
  isLastBatch,
  isLastMessage,
  isStreaming,
  onApprove,
  onDeny,
}) => {
  const hasPendingApproval = tools.some((t) => t.state === 'approval-requested')
  const shouldBeOpen =
    (isLastMessage && isLastBatch && isStreaming) || hasPendingApproval
  const [isOpen, setIsOpen] = useState(shouldBeOpen)
  const [hasUserInteracted, setHasUserInteracted] = useState(false)

  useEffect(() => {
    if (hasPendingApproval) {
      setIsOpen(true)
      return
    }
    if (isLastMessage && !hasUserInteracted) {
      if (isLastBatch) {
        setIsOpen(isStreaming)
      } else {
        setIsOpen(false)
      }
    }
  }, [
    isStreaming,
    isLastMessage,
    isLastBatch,
    hasUserInteracted,
    hasPendingApproval,
  ])

  const completedCount = tools.filter((t) => isToolCompleted(t.state)).length
  const triggerTitle = hasPendingApproval
    ? 'Waiting for approval...'
    : `${completedCount}/${tools.length} actions completed`

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
              <ToolStatusIcon state={tool.state} />
              <span className="flex-1">{formatToolName(tool.toolName)}</span>
            </TaskItem>
            {tool.state === 'approval-requested' &&
              tool.approval?.id != null && (
                <ApprovalButtons
                  approvalId={tool.approval.id}
                  onApprove={onApprove}
                  onDeny={onDeny}
                />
              )}
          </div>
        ))}
      </TaskContent>
    </Task>
  )
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

const isToolApprovalPending = (state: ToolInvocationState) =>
  state === 'approval-requested'

const ApprovalButtons: FC<{
  approvalId: string
  onApprove?: (id: string) => void
  onDeny?: (id: string) => void
}> = ({ approvalId, onApprove, onDeny }) => (
  <div className="mt-1 mb-2 ml-6 flex items-center gap-2">
    <Button
      size="sm"
      className="h-7 gap-1 px-2.5 text-xs"
      onClick={() => onApprove?.(approvalId)}
    >
      <ShieldCheck className="size-3" />
      Approve
    </Button>
    <Button
      size="sm"
      variant="outline"
      className="h-7 gap-1 px-2.5 text-xs"
      onClick={() => onDeny?.(approvalId)}
    >
      <ShieldX className="size-3" />
      Deny
    </Button>
  </div>
)

const ToolStatusIcon: FC<{ state: ToolInvocationState }> = ({ state }) => {
  if (isToolCompleted(state)) {
    return <CheckCircle2 className="h-3.5 w-3.5 text-green-500" />
  }
  if (isToolApprovalPending(state)) {
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
