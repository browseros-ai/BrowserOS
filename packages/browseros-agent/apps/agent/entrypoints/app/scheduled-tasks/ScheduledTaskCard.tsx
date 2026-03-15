import dayjs from 'dayjs'
import duration from 'dayjs/plugin/duration'
import relativeTime from 'dayjs/plugin/relativeTime'
import {
  CheckCircle2,
  ChevronDown,
  Loader2,
  Pencil,
  Play,
  RotateCcw,
  Square,
  Trash2,
  XCircle,
} from 'lucide-react'
import { type FC, useEffect, useMemo, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { Switch } from '@/components/ui/switch'
import { BrowserOSIcon, ProviderIcon } from '@/lib/llm-providers/providerIcons'
import { providersStorage } from '@/lib/llm-providers/storage'
import type { ProviderType } from '@/lib/llm-providers/types'
import { useScheduledJobRuns } from '@/lib/schedules/scheduleStorage'
import type { ScheduledJob, ScheduledJobRun } from './types'

dayjs.extend(relativeTime)
dayjs.extend(duration)

interface ScheduledTaskCardProps {
  job: ScheduledJob
  onEdit: () => void
  onDelete: () => void
  onToggle: (enabled: boolean) => void
  onRun: () => void
  onViewRun: (run: ScheduledJobRun) => void
  onCancelRun: (runId: string) => void
  onRetryRun: (jobId: string) => void
}

function formatSchedule(job: ScheduledJob): string {
  if (job.scheduleType === 'daily' && job.scheduleTime) {
    return `Daily at ${job.scheduleTime}`
  }
  if (job.scheduleType === 'hourly' && job.scheduleInterval) {
    return job.scheduleInterval === 1
      ? 'Every hour'
      : `Every ${job.scheduleInterval} hours`
  }
  if (job.scheduleType === 'minutes' && job.scheduleInterval) {
    return job.scheduleInterval === 1
      ? 'Every minute'
      : `Every ${job.scheduleInterval} minutes`
  }
  return 'Not scheduled'
}

const formatRelativeTime = (dateStr: string) => dayjs(dateStr).fromNow()

function formatDuration(startedAt: string, completedAt?: string): string {
  if (!completedAt) return 'Running...'
  const diff = dayjs(completedAt).diff(dayjs(startedAt))
  const d = dayjs.duration(diff)
  const mins = Math.floor(d.asMinutes())
  const secs = d.seconds()
  if (mins === 0) return `${secs}s`
  return `${mins}m ${secs}s`
}

const formatRunDate = (dateStr: string) =>
  dayjs(dateStr).format('MMM D, h:mm A')

export const ScheduledTaskCard: FC<ScheduledTaskCardProps> = ({
  job,
  onEdit,
  onDelete,
  onToggle,
  onRun,
  onViewRun,
  onCancelRun,
  onRetryRun,
}) => {
  const [isOpen, setIsOpen] = useState(false)
  const [providerInfo, setProviderInfo] = useState<{
    name: string
    type: ProviderType
  } | null>(null)

  const { jobRuns } = useScheduledJobRuns()

  // Load provider info for display
  useEffect(() => {
    if (!job.providerId) {
      setProviderInfo(null)
      return
    }
    providersStorage.getValue().then((providers) => {
      const match = providers?.find((p) => p.id === job.providerId)
      setProviderInfo(match ? { name: match.name, type: match.type } : null)
    })
  }, [job.providerId])

  const runs = useMemo(
    () =>
      jobRuns
        .filter((run) => run.jobId === job.id)
        .sort(
          (a, b) =>
            new Date(b.startedAt).getTime() - new Date(a.startedAt).getTime(),
        ),
    [jobRuns, job.id],
  )

  return (
    <div className="rounded-xl border border-border bg-card p-4 shadow-sm transition-all hover:border-[var(--accent-orange)]/50 hover:shadow-sm">
      <div className="flex items-start gap-4">
        <Switch
          checked={job.enabled}
          onCheckedChange={onToggle}
          aria-label={`${job.enabled ? 'Disable' : 'Enable'} ${job.name}`}
        />

        <div className="min-w-0 flex-1">
          <div className="mb-1 flex items-center gap-2">
            <span className="truncate font-semibold">{job.name}</span>
            {!job.enabled && (
              <span className="rounded bg-muted px-1.5 py-0.5 text-muted-foreground text-xs">
                Disabled
              </span>
            )}
          </div>
          <p className="mb-2 line-clamp-1 text-muted-foreground text-sm">
            "{job.query}"
          </p>
          <div className="flex items-center gap-2 text-muted-foreground text-xs">
            <span>{formatSchedule(job)}</span>
            {providerInfo && (
              <>
                <span>•</span>
                <span className="flex items-center gap-1">
                  {providerInfo.type === 'browseros' ? (
                    <BrowserOSIcon size={12} />
                  ) : (
                    <ProviderIcon type={providerInfo.type} size={12} />
                  )}
                  {providerInfo.name}
                </span>
              </>
            )}
            {job.lastRunAt && (
              <>
                <span>•</span>
                <span>Last run: {formatRelativeTime(job.lastRunAt)}</span>
              </>
            )}
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-2">
          <Button variant="outline" size="sm" onClick={onRun}>
            <Play className="mr-1.5 h-3 w-3" />
            Test
          </Button>
          <Button variant="outline" size="sm" onClick={onEdit}>
            <Pencil className="mr-1.5 h-3 w-3" />
            Edit
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={onDelete}
            className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
            aria-label={`Delete ${job.name}`}
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </div>
      </div>

      {runs.length > 0 && (
        <Collapsible open={isOpen} onOpenChange={setIsOpen} className="mt-4">
          <CollapsibleTrigger className="flex w-full items-center gap-2 text-muted-foreground text-sm hover:text-foreground">
            <ChevronDown
              className={`h-4 w-4 transition-transform duration-200 ${
                isOpen ? 'rotate-180' : ''
              }`}
            />
            <span>Run History ({runs.length})</span>
          </CollapsibleTrigger>
          <CollapsibleContent className="pt-3">
            <div className="space-y-2">
              {runs.map((run) => (
                <div
                  key={run.id}
                  className="flex items-center gap-3 rounded-lg border border-border bg-background p-3"
                >
                  {run.status === 'completed' ? (
                    <CheckCircle2 className="h-4 w-4 shrink-0 text-green-500" />
                  ) : run.status === 'failed' ? (
                    <XCircle className="h-4 w-4 shrink-0 text-destructive" />
                  ) : (
                    <Loader2 className="h-4 w-4 shrink-0 animate-spin text-accent-orange" />
                  )}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="text-sm">
                        {formatRunDate(run.startedAt)}
                      </span>
                      <span className="text-muted-foreground text-xs">
                        {formatDuration(run.startedAt, run.completedAt)}
                      </span>
                    </div>
                    {run.status === 'failed' && run.result && (
                      <p className="mt-1 line-clamp-1 text-destructive text-xs">
                        {run.result}
                      </p>
                    )}
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    {run.status === 'running' && (
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        onClick={(e) => {
                          e.stopPropagation()
                          onCancelRun(run.id)
                        }}
                        className="text-destructive hover:bg-destructive/10 hover:text-destructive"
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
                        className="text-muted-foreground hover:text-foreground"
                        aria-label="Retry run"
                      >
                        <RotateCcw className="h-3.5 w-3.5" />
                      </Button>
                    )}
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => onViewRun(run)}
                      className="shrink-0 text-muted-foreground hover:text-foreground"
                    >
                      View
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          </CollapsibleContent>
        </Collapsible>
      )}
    </div>
  )
}
