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
  let latest: number | null = null

  const candidates = [
    job.lastRunAt,
    ...runs.filter((run) => run.jobId === job.id).map((run) => run.startedAt),
  ]
  for (const candidate of candidates) {
    if (!candidate) continue
    const time = new Date(candidate).getTime()
    if (Number.isFinite(time) && (latest === null || time > latest)) {
      latest = time
    }
  }

  return latest
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
