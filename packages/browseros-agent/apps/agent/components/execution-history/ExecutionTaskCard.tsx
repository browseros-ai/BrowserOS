import dayjs from 'dayjs'
import duration from 'dayjs/plugin/duration'
import relativeTime from 'dayjs/plugin/relativeTime'
import {
  CheckCircle2,
  ChevronDown,
  CircleDot,
  CircleSlash2,
  MessageSquareText,
  Trash2,
  XCircle,
} from 'lucide-react'
import { type FC, useMemo, useState } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import type { ExecutionTaskRecord } from '@/lib/execution-history/types'
import { cn } from '@/lib/utils'
import { ExecutionStepItem } from './ExecutionStepItem'

dayjs.extend(relativeTime)
dayjs.extend(duration)

function getTaskStatusIcon(status: ExecutionTaskRecord['status']) {
  if (status === 'completed') {
    return <CheckCircle2 className="h-4 w-4 text-green-500" />
  }

  if (status === 'running') {
    return <CircleDot className="h-4 w-4 text-[var(--accent-orange)]" />
  }

  if (status === 'stopped') {
    return <CircleSlash2 className="h-4 w-4 text-orange-500" />
  }

  return <XCircle className="h-4 w-4 text-destructive" />
}

function getTaskStatusLabel(status: ExecutionTaskRecord['status']) {
  if (status === 'completed') return 'Completed'
  if (status === 'running') return 'Running'
  if (status === 'stopped') return 'Stopped'
  if (status === 'interrupted') return 'Interrupted'
  return 'Failed'
}

function formatDuration(task: ExecutionTaskRecord): string | null {
  if (!task.completedAt) return null
  const diff = dayjs(task.completedAt).diff(task.startedAt)
  const parsed = dayjs.duration(diff)
  const minutes = Math.floor(parsed.asMinutes())
  const seconds = parsed.seconds()
  if (minutes === 0) return `${seconds}s`
  return `${minutes}m ${seconds}s`
}

export const ExecutionTaskCard: FC<{
  task: ExecutionTaskRecord
  defaultOpen?: boolean
  onDelete?: (task: ExecutionTaskRecord) => void
}> = ({ task, defaultOpen = false, onDelete }) => {
  const [open, setOpen] = useState(defaultOpen)
  const startedAgo = useMemo(
    () => dayjs(task.startedAt).fromNow(),
    [task.startedAt],
  )

  return (
    <Collapsible open={open} onOpenChange={setOpen}>
      <div className="rounded-2xl border border-border/60 bg-card shadow-sm">
        <div className="flex items-start gap-2 px-5 py-5">
          <CollapsibleTrigger asChild>
            <button
              type="button"
              className="flex min-w-0 flex-1 items-start gap-3 text-left"
            >
              <div className="mt-0.5 shrink-0">
                {getTaskStatusIcon(task.status)}
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2">
                  <p className="line-clamp-2 font-medium text-base text-foreground">
                    {task.promptText}
                  </p>
                  <Badge variant="secondary">
                    {getTaskStatusLabel(task.status)}
                  </Badge>
                </div>
                <div className="mt-2 flex flex-wrap items-center gap-2 text-muted-foreground text-xs">
                  <span>{startedAgo}</span>
                  <span>•</span>
                  <span>
                    {task.actionCount} action{task.actionCount === 1 ? '' : 's'}
                  </span>
                  {formatDuration(task) && (
                    <>
                      <span>•</span>
                      <span>{formatDuration(task)}</span>
                    </>
                  )}
                  {task.deniedCount > 0 && (
                    <Badge variant="outline" className="h-5 rounded-full px-2">
                      {task.deniedCount} denied
                    </Badge>
                  )}
                  {task.errorCount > 0 && (
                    <Badge variant="outline" className="h-5 rounded-full px-2">
                      {task.errorCount} error
                      {task.errorCount === 1 ? '' : 's'}
                    </Badge>
                  )}
                </div>
                {task.responsePreview ? (
                  <div className="mt-4 flex items-start gap-2 rounded-xl bg-muted/40 px-3 py-2 text-muted-foreground text-sm">
                    <MessageSquareText className="mt-0.5 h-4 w-4 shrink-0" />
                    <p className="line-clamp-2">{task.responsePreview}</p>
                  </div>
                ) : null}
              </div>
              <ChevronDown
                className={cn(
                  'mt-1 h-4 w-4 shrink-0 text-muted-foreground transition-transform',
                  open && 'rotate-180',
                )}
              />
            </button>
          </CollapsibleTrigger>
          {onDelete ? (
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="mt-0.5 shrink-0 text-muted-foreground hover:text-foreground"
              onClick={() => onDelete(task)}
              aria-label={`Delete ${task.promptText}`}
            >
              <Trash2 className="size-4" />
            </Button>
          ) : null}
        </div>
        <CollapsibleContent className="border-border/60 border-t px-5 py-5">
          {task.steps.length === 0 ? (
            <div className="rounded-xl border border-border/70 border-dashed bg-muted/30 px-4 py-6 text-center text-muted-foreground text-sm">
              No tool actions were recorded for this task.
            </div>
          ) : (
            <div className="space-y-3">
              {task.steps.map((step, index) => (
                <ExecutionStepItem
                  key={step.id}
                  step={step}
                  defaultOpen={
                    task.status === 'running' && index === task.steps.length - 1
                  }
                />
              ))}
            </div>
          )}
        </CollapsibleContent>
      </div>
    </Collapsible>
  )
}
