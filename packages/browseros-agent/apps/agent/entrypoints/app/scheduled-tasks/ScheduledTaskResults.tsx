import { Calendar } from 'lucide-react'
import type { FC } from 'react'
import { useMemo, useState } from 'react'
import {
  useScheduledJobRuns,
  useScheduledJobs,
} from '@/lib/schedules/scheduleStorage'
import type { ScheduledJobRun } from '@/lib/schedules/scheduleTypes'
import { ScheduledTaskResultGroup } from './ScheduledTaskResultGroup'
import { groupRunsByJob } from './types'

interface ScheduledTaskResultsProps {
  onViewRun: (run: ScheduledJobRun) => void
  onCancelRun: (runId: string) => void
  onRetryRun: (jobId: string) => void
}

export const ScheduledTaskResults: FC<ScheduledTaskResultsProps> = ({
  onViewRun,
  onCancelRun,
  onRetryRun,
}) => {
  const { jobRuns } = useScheduledJobRuns()
  const { jobs } = useScheduledJobs()
  const [openGroupId, setOpenGroupId] = useState<string | null>(null)

  const groupedRuns = useMemo(
    () => groupRunsByJob(jobRuns, jobs),
    [jobRuns, jobs],
  )

  if (!groupedRuns.length) {
    return (
      <div className="flex flex-col items-center justify-center gap-3 py-16 text-muted-foreground">
        <Calendar className="h-10 w-10 opacity-50" />
        <p className="text-sm">No task runs yet</p>
      </div>
    )
  }

  return (
    <div className="space-y-2">
      {groupedRuns.map(({ job, runs }) => (
        <ScheduledTaskResultGroup
          key={job.id}
          job={job}
          runs={runs}
          isOpen={openGroupId === job.id}
          onOpenChange={(open) => setOpenGroupId(open ? job.id : null)}
          onViewRun={onViewRun}
          onCancelRun={onCancelRun}
          onRetryRun={onRetryRun}
        />
      ))}
    </div>
  )
}
