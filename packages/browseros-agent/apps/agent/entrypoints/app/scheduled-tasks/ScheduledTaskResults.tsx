import { Calendar } from 'lucide-react'
import type { FC } from 'react'
import { useMemo, useState } from 'react'
import {
  useScheduledJobRuns,
  useScheduledJobs,
} from '@/lib/schedules/scheduleStorage'
import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'
import { ScheduledTaskResultGroup } from './ScheduledTaskResultGroup'

interface JobRunWithDetails extends ScheduledJobRun {
  job: ScheduledJob | undefined
}

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

  const groupedRuns = useMemo(() => {
    const enrichWithJob = (run: ScheduledJobRun): JobRunWithDetails => ({
      ...run,
      job: jobs.find((j) => j.id === run.jobId),
    })

    const runsByJob = new Map<string, JobRunWithDetails[]>()

    for (const run of jobRuns) {
      const enriched = enrichWithJob(run)
      const existing = runsByJob.get(run.jobId) ?? []
      existing.push(enriched)
      runsByJob.set(run.jobId, existing)
    }

    const groups: Array<{ job: ScheduledJob; runs: JobRunWithDetails[] }> = []

    for (const [jobId, runs] of runsByJob) {
      const job = jobs.find((j) => j.id === jobId)
      if (!job) continue

      const sorted = runs.sort(
        (a, b) =>
          new Date(b.startedAt).getTime() - new Date(a.startedAt).getTime(),
      )

      groups.push({ job, runs: sorted })
    }

    return groups.sort((a, b) => {
      const latestA = a.runs[0]?.startedAt ?? ''
      const latestB = b.runs[0]?.startedAt ?? ''
      return new Date(latestB).getTime() - new Date(latestA).getTime()
    })
  }, [jobRuns, jobs])

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
