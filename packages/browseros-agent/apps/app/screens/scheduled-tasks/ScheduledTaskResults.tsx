import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'
import {
  Calendar,
  CheckCircle2,
  Clock,
  Loader2,
  RotateCcw,
  Square,
  XCircle,
} from 'lucide-react'
import type { FC } from 'react'
import { useMemo } from 'react'
import { Button } from '@/components/ui/button'
import {
  useScheduledJobRuns,
  useScheduledJobs,
} from '@/lib/schedules/scheduleStorage'
import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'

dayjs.extend(relativeTime)

interface JobRunWithDetails extends ScheduledJobRun {
  job: ScheduledJob | undefined
}

export interface ScheduledTaskResultsProps {
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

export const ScheduledTaskResults: FC<ScheduledTaskResultsProps> = ({
  onViewRun,
  onCancelRun,
  onRetryRun,
}) => {
  const { jobRuns } = useScheduledJobRuns()
  const { jobs } = useScheduledJobs()

  const sortedRuns: JobRunWithDetails[] = useMemo(() => {
    const enrichWithJob = (run: ScheduledJobRun): JobRunWithDetails => ({
      ...run,
      job: jobs.find((j) => j.id === run.jobId),
    })

    const running = jobRuns
      .filter((r) => r.status === 'running')
      .map(enrichWithJob)

    const completedOrFailed = jobRuns
      .filter((r) => r.status === 'completed' || r.status === 'failed')
      .sort(
        (a, b) =>
          new Date(b.startedAt).getTime() - new Date(a.startedAt).getTime(),
      )
      .map(enrichWithJob)

    return [...running, ...completedOrFailed]
  }, [jobRuns, jobs])

  if (!sortedRuns.length) {
    return (
      <div className="flex flex-col items-center justify-center gap-3 py-16 text-muted-foreground">
        <Calendar className="h-10 w-10 opacity-50" />
        <p className="text-sm">No task runs yet</p>
      </div>
    )
  }

  return (
    <div className="space-y-2">
      {sortedRuns.map((run) => (
        <Button
          key={run.id}
          variant="ghost"
          onClick={() => onViewRun(run)}
          className="h-auto w-full justify-start rounded-xl border border-border/50 bg-card p-4 text-left transition-all hover:border-border"
        >
          <div className="flex w-full items-start gap-3">
            {getStatusIcon(run.status)}
            <div className="min-w-0 flex-1">
              <div className="mb-1 flex items-center gap-2">
                <span className="truncate font-medium text-foreground text-sm">
                  {run.job?.name}
                </span>
                <span className="flex items-center gap-1 text-muted-foreground text-xs">
                  <Clock className="h-3 w-3" />
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
  )
}
