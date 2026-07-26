import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'

export interface JobRunWithDetails extends ScheduledJobRun {
  job: ScheduledJob | undefined
}

export function countRunningRuns(runs: readonly ScheduledJobRun[]): number {
  return runs.filter((run) => run.status === 'running').length
}

/** Pick the runs the home widget shows: all in-flight runs first, then the most
 * recent finished runs to fill up to `max`. When running runs already meet
 * `max`, every running run is kept (so nothing in-flight is hidden) and no
 * finished runs are added. Each run is paired with its owning job. */
export function selectDisplayedRuns(
  runs: readonly ScheduledJobRun[],
  jobs: readonly ScheduledJob[],
  max: number,
): JobRunWithDetails[] {
  const enrich = (run: ScheduledJobRun): JobRunWithDetails => ({
    ...run,
    job: jobs.find((job) => job.id === run.jobId),
  })

  const running = runs.filter((run) => run.status === 'running').map(enrich)
  if (running.length >= max) return running

  const finished = runs
    .filter((run) => run.status === 'completed' || run.status === 'failed')
    .sort(
      (a, b) =>
        new Date(b.startedAt).getTime() - new Date(a.startedAt).getTime(),
    )
    .slice(0, max - running.length)
    .map(enrich)

  return [...running, ...finished]
}
