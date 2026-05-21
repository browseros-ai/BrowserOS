import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'
import {
  Calendar,
  CheckCircle2,
  Clock,
  Loader2,
  RotateCcw,
  Square,
  Trash2,
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

interface ScheduledTaskResultsProps {
  onViewRun: (run: ScheduledJobRun) => void
  onCancelRun: (runId: string) => void
  onRetryRun: (jobId: string) => void
  onRemoveRun: (runId: string) => void
  onClearAll: () => void
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
  onRemoveRun,
  onClearAll,
}) => {
  const { jobRuns } = useScheduledJobRuns()
  const { jobs } = useScheduledJobs()

  const groupedRuns = useMemo(() => {
    const enrichWithJob = (run: ScheduledJobRun): JobRunWithDetails => ({
      ...run,
      job: jobs.find((j) => j.id === run.jobId),
    })

    const sorted = [...jobRuns].sort(
      (a, b) =>
        new Date(b.startedAt).getTime() - new Date(a.startedAt).getTime(),
    )

    const groups: Record<string, JobRunWithDetails[]> = {}

    for (const run of sorted) {
      const date = dayjs(run.startedAt)
      let groupTitle = ''

      if (date.isSame(dayjs(), 'day')) {
        groupTitle = 'Today'
      } else if (date.isSame(dayjs().subtract(1, 'day'), 'day')) {
        groupTitle = 'Yesterday'
      } else if (date.isAfter(dayjs().subtract(7, 'days'))) {
        groupTitle = 'Last 7 Days'
      } else {
        groupTitle = date.format('MMMM D, YYYY')
      }

      if (!groups[groupTitle]) {
        groups[groupTitle] = []
      }
      groups[groupTitle].push(enrichWithJob(run))
    }

    return Object.entries(groups)
  }, [jobRuns, jobs])

  if (!jobRuns.length) {
    return (
      <div className="flex flex-col items-center justify-center gap-3 py-16 text-muted-foreground">
        <Calendar className="h-10 w-10 opacity-50" />
        <p className="text-sm">No task runs yet</p>
      </div>
    )
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h3 className="font-medium text-muted-foreground text-xs uppercase tracking-wider">
          History
        </h3>
        <Button
          variant="ghost"
          size="sm"
          onClick={onClearAll}
          className="h-8 text-muted-foreground hover:text-destructive"
        >
          Clear All
        </Button>
      </div>

      {groupedRuns.map(([title, runs]) => (
        <div key={title} className="space-y-2">
          <h4 className="px-1 font-medium text-muted-foreground text-xs">
            {title}
          </h4>
          <div className="space-y-2">
            {runs.map((run) => (
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
                        {run.job?.name || 'Unknown Task'}
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
                  <div className="flex items-center gap-1">
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
                    {run.status !== 'running' && (
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={(e) => {
                          e.stopPropagation()
                          onRemoveRun(run.id)
                        }}
                        className="shrink-0 text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                        aria-label="Remove result"
                      >
                        <Trash2 className="h-3.5 w-3.5" />
                      </Button>
                    )}
                  </div>
                </div>
              </Button>
            ))}
          </div>
        </div>
      ))}
    </div>
  )
}

