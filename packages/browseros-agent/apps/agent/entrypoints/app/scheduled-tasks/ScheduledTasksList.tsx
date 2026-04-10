import type { FC } from 'react'
import { ScheduledTaskCard } from './ScheduledTaskCard'
import type { ScheduledJob, ScheduledJobRun } from './types'

interface ScheduledTasksListProps {
  jobs: ScheduledJob[]
  onEdit: (job: ScheduledJob) => void
  onDelete: (jobId: string) => void
  onToggle: (jobId: string, enabled: boolean) => void
  onRun: (jobId: string) => void
  onViewRun: (run: ScheduledJobRun) => void
  onCancelRun: (runId: string) => void
  onRetryRun: (jobId: string) => void
}

export const ScheduledTasksList: FC<ScheduledTasksListProps> = ({
  jobs,
  onEdit,
  onDelete,
  onToggle,
  onRun,
  onViewRun,
  onCancelRun,
  onRetryRun,
}) => {
  if (jobs.length === 0) {
    return (
      <div className="rounded-xl border border-border bg-card p-6 shadow-sm">
        <div className="rounded-lg border border-border border-dashed py-8 text-center">
          <p className="text-muted-foreground text-sm">
            No scheduled tasks yet. Create one to automate recurring tasks.
          </p>
        </div>
      </div>
    )
  }

  return (
    <div className="space-y-3">
      {jobs.map((job) => (
        <ScheduledTaskCard
          key={job.id}
          job={job}
          onEdit={() => onEdit(job)}
          onDelete={() => onDelete(job.id)}
          onToggle={(enabled) => onToggle(job.id, enabled)}
          onRun={() => onRun(job.id)}
          onViewRun={onViewRun}
          onCancelRun={onCancelRun}
          onRetryRun={onRetryRun}
        />
      ))}
    </div>
  )
}
