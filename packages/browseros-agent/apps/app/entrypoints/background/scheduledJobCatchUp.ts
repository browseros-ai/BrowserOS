import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'

const hourMs = 60 * 60 * 1000
const minuteMs = 60 * 1000

function latestRunTime(
  job: ScheduledJob,
  runs: ScheduledJobRun[],
): number | null {
  // Prefer the latest start time from real run history. `lastRunAt` is written
  // when a run *finishes*, so letting it win would hide a missed run whenever
  // the previous one ran long: an hourly job that starts at 10:00 and finishes
  // at 10:20 would still look current at 11:10 even though its 11:00 run never
  // fired.
  let latest: number | null = null
  for (const run of runs) {
    if (run.jobId !== job.id) continue
    const time = new Date(run.startedAt).getTime()
    if (Number.isFinite(time) && (latest === null || time > latest)) {
      latest = time
    }
  }
  if (latest !== null) return latest

  // Only when run history is unavailable does the job's own bookkeeping stand in.
  if (!job.lastRunAt) return null
  const fallback = new Date(job.lastRunAt).getTime()
  return Number.isFinite(fallback) ? fallback : null
}

/** Whether a startup pass should run this job after missed alarms. */
export function shouldCatchUpScheduledJob(
  job: ScheduledJob,
  runs: ScheduledJobRun[],
  now: number,
): boolean {
  if (!job.enabled) return false
  if (runs.some((run) => run.jobId === job.id && run.status === 'running')) {
    return false
  }

  const lastRun = latestRunTime(job, runs)

  if (job.scheduleType === 'hourly' || job.scheduleType === 'minutes') {
    if (!job.scheduleInterval || job.scheduleInterval <= 0) return false
    const intervalMs =
      job.scheduleType === 'hourly'
        ? job.scheduleInterval * hourMs
        : job.scheduleInterval * minuteMs
    const origin = lastRun ?? new Date(job.createdAt).getTime()
    return Number.isFinite(origin) && now - origin >= intervalMs
  }

  if (job.scheduleType === 'daily' && job.scheduleTime) {
    const [hours, minutes] = job.scheduleTime.split(':').map(Number)
    if (!Number.isInteger(hours) || !Number.isInteger(minutes)) return false

    const scheduledToday = new Date(now)
    scheduledToday.setHours(hours, minutes, 0, 0)
    const scheduledAt = scheduledToday.getTime()
    if (now < scheduledAt) return false

    return lastRun === null || lastRun < scheduledAt
  }

  return false
}
