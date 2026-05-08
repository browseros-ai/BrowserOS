import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'
import {
  CheckCircle2,
  ChevronDown,
  Clock,
  Loader2,
  RotateCcw,
  Square,
  XCircle,
} from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import type { JobRunWithDetails, ScheduledJob, ScheduledJobRun } from './types'

dayjs.extend(relativeTime)

interface ScheduledTaskResultGroupProps {
  job: ScheduledJob
  runs: JobRunWithDetails[]
  isOpen: boolean
  onOpenChange: (open: boolean) => void
  onViewRun: (run: ScheduledJobRun) => void
  onCancelRun: (runId: string) => void
  onRetryRun: (jobId: string) => void
}

const getStatusIcon = (status: JobRunWithDetails['status']) => {
  switch (status) {
    case 'completed':
      return <CheckCircle2 className="h-4 w-4 text-green-500" />
    case 'running':
      return <Loader2 className="h-4 w-4 animate-spin text-accent-orange" />
    case 'failed':
      return <XCircle className="h-4 w-4 text-destructive" />
  }
}

const formatTimestamp = (dateString: string) => dayjs(dateString).fromNow()

export const ScheduledTaskResultGroup: FC<ScheduledTaskResultGroupProps> = ({
  job,
  runs,
  isOpen,
  onOpenChange,
  onViewRun,
  onCancelRun,
  onRetryRun,
}) => {
  const latestRun = runs[0]
  const latestTime = latestRun ? formatTimestamp(latestRun.startedAt) : ''
  const runningCount = runs.filter((r) => r.status === 'running').length

  return (
    <Collapsible open={isOpen} onOpenChange={onOpenChange}>
      <CollapsibleTrigger asChild>
        <Button
          variant="ghost"
          className="group flex h-auto w-full items-center justify-between rounded-xl border border-border/50 bg-card p-4 text-left transition-all hover:border-border"
        >
          <div className="flex items-center gap-3">
            {latestRun ? getStatusIcon(latestRun.status) : null}
            <div className="min-w-0 flex-1">
              <div className="mb-1 flex items-center gap-2">
                <span className="truncate font-medium text-foreground text-sm">
                  {job.name}
                </span>
                {runningCount > 0 && (
                  <span className="rounded-full bg-accent-orange/20 px-2 py-0.5 text-accent-orange text-xs">
                    {runningCount} running
                  </span>
                )}
              </div>
              <div className="flex items-center gap-2 text-muted-foreground text-xs">
                <span className="flex items-center gap-1">
                  <Clock className="h-3 w-3" />
                  {latestTime}
                </span>
                <span>•</span>
                <span>
                  {runs.length} {runs.length === 1 ? 'result' : 'results'}
                </span>
              </div>
            </div>
          </div>
          <ChevronDown
            className={`h-4 w-4 text-muted-foreground transition-transform duration-200 ${isOpen ? 'rotate-180' : ''}`}
          />
        </Button>
      </CollapsibleTrigger>

      <CollapsibleContent className="fade-in-0 slide-in-from-top-2 animate-in pt-2 duration-200">
        <div className="space-y-2 pl-4">
          {runs.map((run) => (
            <Button
              key={run.id}
              variant="ghost"
              onClick={() => onViewRun(run)}
              className="h-auto w-full justify-start rounded-lg border border-border/50 bg-background p-3 text-left transition-all hover:border-border"
            >
              <div className="flex w-full items-start gap-3">
                {getStatusIcon(run.status)}
                <div className="min-w-0 flex-1">
                  <div className="mb-1 flex items-center gap-2">
                    <span className="text-sm">
                      {dayjs(run.startedAt).format('MMM D, h:mm A')}
                    </span>
                    <span className="flex items-center gap-1 text-muted-foreground text-xs">
                      {formatTimestamp(run.startedAt)}
                    </span>
                  </div>
                  {run.result && (
                    <p className="line-clamp-2 text-ellipsis text-muted-foreground text-xs">
                      {run.result}
                    </p>
                  )}
                </div>
                {run.status === 'running' && (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={(e) => {
                      e.stopPropagation()
                      onCancelRun(run.id)
                    }}
                    className="shrink-0 text-destructive hover:bg-destructive/10 hover:text-destructive"
                    aria-label="Cancel run"
                  >
                    <Square className="h-3.5 w-3.5" />
                  </Button>
                )}
                {run.status === 'failed' && (
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    onClick={(e) => {
                      e.stopPropagation()
                      onRetryRun(run.jobId)
                    }}
                    className="shrink-0 text-muted-foreground hover:text-foreground"
                    aria-label="Retry run"
                  >
                    <RotateCcw className="h-3.5 w-3.5" />
                  </Button>
                )}
              </div>
            </Button>
          ))}
        </div>
      </CollapsibleContent>
    </Collapsible>
  )
}
