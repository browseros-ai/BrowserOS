import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'
import {
  Calendar,
  CheckCircle2,
  ChevronDown,
  Clock,
  Loader2,
  RotateCcw,
  Square,
  XCircle,
} from 'lucide-react'
import type { FC } from 'react'
import { useMemo, useState } from 'react'
import { RunResultDialog } from '@/components/ai-elements/run-result-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import {
  SCHEDULED_TASK_CANCELLED_EVENT,
  SCHEDULED_TASK_RETRIED_EVENT,
  SCHEDULED_TASK_VIEW_MORE_IN_NEWTAB_EVENT,
  SCHEDULED_TASK_VIEW_RESULTS_IN_NEWTAB_EVENT,
} from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import {
  useScheduledJobRuns,
  useScheduledJobs,
} from '@/lib/schedules/scheduleStorage'
import {
  groupRunsByJob,
  type JobGroup,
  type JobRunWithDetails,
} from '../../app/scheduled-tasks/types'

dayjs.extend(relativeTime)

const MAX_DISPLAY_COUNT = 3
const SCHEDULE_RESULTS_COLLAPSED_KEY = 'schedule-results-collapsed'

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

export const ScheduleResults: FC = () => {
  const [isOpen, setIsOpen] = useState(() => {
    const stored = localStorage.getItem(SCHEDULE_RESULTS_COLLAPSED_KEY)
    return stored !== 'true'
  })
  const [openGroupId, setOpenGroupId] = useState<string | null>(null)
  const [viewingRun, setViewingRun] = useState<JobRunWithDetails | null>(null)

  const handleOpenChange = (open: boolean) => {
    setIsOpen(open)
    localStorage.setItem(SCHEDULE_RESULTS_COLLAPSED_KEY, (!open).toString())
  }

  const { jobRuns, cancelJobRun } = useScheduledJobRuns()
  const { jobs, runJob } = useScheduledJobs()

  const runningCount = jobRuns.filter((r) => r.status === 'running').length

  const groupedRuns: JobGroup[] = useMemo(() => {
    const allGroups = groupRunsByJob(jobRuns, jobs)

    const withRunning = allGroups.filter((g) =>
      g.runs.some((r) => r.status === 'running'),
    )
    const withoutRunning = allGroups.filter(
      (g) => !g.runs.some((r) => r.status === 'running'),
    )

    const result = [...withRunning]
    for (const group of withoutRunning) {
      if (result.length >= MAX_DISPLAY_COUNT) break
      result.push(group)
    }

    return result
  }, [jobRuns, jobs])

  const viewRun = (run: JobRunWithDetails) => {
    track(SCHEDULED_TASK_VIEW_RESULTS_IN_NEWTAB_EVENT)
    setViewingRun(run)
  }

  const handleCancelRun = async (runId: string) => {
    await cancelJobRun(runId)
    track(SCHEDULED_TASK_CANCELLED_EVENT)
  }

  const handleRetryRun = async (jobId: string) => {
    await runJob(jobId)
    setViewingRun(null)
    track(SCHEDULED_TASK_RETRIED_EVENT)
  }

  if (!groupedRuns.length) return null

  return (
    <Collapsible
      open={isOpen}
      onOpenChange={handleOpenChange}
      className="mx-auto mb-16 w-lg space-y-3 md:w-2xl"
    >
      <CollapsibleTrigger asChild>
        <Button
          variant="ghost"
          className="group flex h-auto w-full items-center justify-between rounded-xl border border-border/50 bg-card/50 p-3 transition-all hover:border-border hover:bg-card"
        >
          <div className="flex items-center gap-3">
            <Calendar className="h-4 w-4 text-muted-foreground" />
            <span className="font-medium text-foreground text-sm">
              Scheduled Task Outputs
            </span>
            {runningCount > 0 && (
              <Badge variant="secondary" className="text-xs">
                {runningCount} running
              </Badge>
            )}
          </div>
          <ChevronDown
            className={`h-4 w-4 text-muted-foreground transition-transform duration-200 ${isOpen ? 'rotate-180' : ''}`}
          />
        </Button>
      </CollapsibleTrigger>

      <CollapsibleContent className="fade-in-0 slide-in-from-top-2 animate-in space-y-2 duration-200">
        {groupedRuns.map(({ job, runs }) => {
          const latestRun = runs[0]
          const latestTime = latestRun
            ? formatTimestamp(latestRun.startedAt)
            : ''
          const groupRunningCount = runs.filter(
            (r) => r.status === 'running',
          ).length

          return (
            <Collapsible
              key={job.id}
              open={openGroupId === job.id}
              onOpenChange={(open) => setOpenGroupId(open ? job.id : null)}
            >
              <CollapsibleTrigger asChild>
                <Button
                  variant="ghost"
                  className="flex h-auto w-full items-center justify-between rounded-xl border border-border/50 bg-card p-4 text-left transition-all hover:border-border"
                >
                  <div className="flex items-center gap-3">
                    {latestRun ? getStatusIcon(latestRun.status) : null}
                    <div className="min-w-0 flex-1">
                      <div className="mb-1 flex items-center gap-2">
                        <span className="truncate font-medium text-foreground text-sm">
                          {job.name}
                        </span>
                        {groupRunningCount > 0 && (
                          <span className="rounded-full bg-accent-orange/20 px-2 py-0.5 text-accent-orange text-xs">
                            {groupRunningCount} running
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
                          {runs.length}{' '}
                          {runs.length === 1 ? 'result' : 'results'}
                        </span>
                      </div>
                    </div>
                  </div>
                  <ChevronDown
                    className={`h-4 w-4 text-muted-foreground transition-transform duration-200 ${openGroupId === job.id ? 'rotate-180' : ''}`}
                  />
                </Button>
              </CollapsibleTrigger>

              <CollapsibleContent className="fade-in-0 slide-in-from-top-2 animate-in pt-2 duration-200">
                <div className="space-y-2 pl-4">
                  {runs.map((run) => (
                    <Button
                      key={run.id}
                      variant="ghost"
                      onClick={() => viewRun(run)}
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
                              handleCancelRun(run.id)
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
                              handleRetryRun(run.jobId)
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
        })}
        <Button variant="ghost" asChild className="w-full">
          {/* biome-ignore lint/a11y/useValidAnchor: click handler is passive */}
          <a
            href="/app.html#/scheduled"
            onClick={() => track(SCHEDULED_TASK_VIEW_MORE_IN_NEWTAB_EVENT)}
          >
            View more
          </a>
        </Button>
      </CollapsibleContent>

      <RunResultDialog
        run={viewingRun}
        jobName={viewingRun?.job?.name}
        onOpenChange={(open) => !open && setViewingRun(null)}
        onCancelRun={handleCancelRun}
        onRetryRun={handleRetryRun}
      />
    </Collapsible>
  )
}
