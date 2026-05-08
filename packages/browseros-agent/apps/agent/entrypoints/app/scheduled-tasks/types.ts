import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'

export type { ScheduledJob, ScheduledJobRun }

export interface ScheduledTasksStorage {
  loadJobs(): Promise<ScheduledJob[]>
  saveJobs(jobs: ScheduledJob[]): Promise<void>
  loadRuns(): Promise<ScheduledJobRun[]>
  saveRuns(runs: ScheduledJobRun[]): Promise<void>
}

export interface JobRunWithDetails extends ScheduledJobRun {
  job: ScheduledJob
}

export interface JobGroup {
  job: ScheduledJob
  runs: JobRunWithDetails[]
}

export function groupRunsByJob(
  jobRuns: ScheduledJobRun[],
  jobs: ScheduledJob[],
): JobGroup[] {
  const jobsById = new Map(jobs.map((j) => [j.id, j]))
  const runsByJob = new Map<string, JobRunWithDetails[]>()

  for (const run of jobRuns) {
    const job = jobsById.get(run.jobId) ?? {
      id: run.jobId,
      name: 'Unknown task',
      query: '',
      scheduleType: 'daily' as const,
      enabled: false,
      createdAt: '',
      updatedAt: '',
    }

    const enriched = { ...run, job }
    const existing = runsByJob.get(run.jobId) ?? []
    existing.push(enriched)
    runsByJob.set(run.jobId, existing)
  }

  const groups: JobGroup[] = []

  for (const [, runs] of runsByJob) {
    const sorted = runs.sort(
      (a, b) =>
        new Date(b.startedAt).getTime() - new Date(a.startedAt).getTime(),
    )
    groups.push({ job: sorted[0].job, runs: sorted })
  }

  return groups.sort((a, b) => {
    const latestA = a.runs[0]?.startedAt ?? ''
    const latestB = b.runs[0]?.startedAt ?? ''
    return new Date(latestB).getTime() - new Date(latestA).getTime()
  })
}
