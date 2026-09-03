import type {
  ScheduledJob,
  ScheduledJobRun,
  ToolCallExecution,
} from '@/lib/schedules/scheduleTypes'

/** A job row as the server returns it: absent values are null, times are epoch. */
export interface ScheduledJobRow {
  id: string
  name: string
  query: string
  scheduleType: ScheduledJob['scheduleType']
  scheduleTime: string | null
  scheduleInterval: number | null
  enabled: boolean
  providerId: string | null
  lastRunAt: number | null
  createdAt: number
  updatedAt: number
}

/** A run row as the server returns it. */
export interface ScheduledJobRunRow {
  id: string
  jobId: string
  status: ScheduledJobRun['status']
  startedAt: number
  completedAt: number | null
  result: string | null
  finalResult: string | null
  executionLog: string | null
  toolCalls: ToolCallExecution[] | null
  error: string | null
}

function orUndefined<T>(value: T | null): T | undefined {
  return value ?? undefined
}

function toIso(value: number | null): string | undefined {
  return value === null ? undefined : new Date(value).toISOString()
}

function toEpoch(value: string | undefined): number | undefined {
  if (!value) return undefined
  const parsed = Date.parse(value)
  return Number.isNaN(parsed) ? undefined : parsed
}

export function toScheduledJob(row: ScheduledJobRow): ScheduledJob {
  return {
    id: row.id,
    name: row.name,
    query: row.query,
    scheduleType: row.scheduleType,
    scheduleTime: orUndefined(row.scheduleTime),
    scheduleInterval: orUndefined(row.scheduleInterval),
    enabled: row.enabled,
    providerId: orUndefined(row.providerId),
    lastRunAt: toIso(row.lastRunAt),
    // Not nullable in the database, so these always convert.
    createdAt: new Date(row.createdAt).toISOString(),
    updatedAt: new Date(row.updatedAt).toISOString(),
  }
}

/** The request body for a job write. `id` travels in the path instead. */
export function toScheduledJobPayload(job: ScheduledJob) {
  return {
    name: job.name,
    query: job.query,
    scheduleType: job.scheduleType,
    scheduleTime: job.scheduleTime,
    scheduleInterval: job.scheduleInterval,
    enabled: job.enabled,
    providerId: job.providerId,
    lastRunAt: toEpoch(job.lastRunAt),
    createdAt: toEpoch(job.createdAt),
  }
}

/**
 * The job to write back when recording that a run finished.
 *
 * Takes the current list rather than a job captured earlier: a run can last
 * minutes, and the user can rename, reschedule, disable or repoint the job
 * while it goes. Writing an earlier copy back would revert all of it.
 *
 * Returns null when the job was deleted during the run, so finishing does not
 * resurrect it.
 */
export function applyLastRunAt(
  jobs: readonly ScheduledJob[],
  jobId: string,
  at: string,
): ScheduledJob | null {
  const job = jobs.find((each) => each.id === jobId)
  return job ? { ...job, lastRunAt: at } : null
}

export function toScheduledJobRun(row: ScheduledJobRunRow): ScheduledJobRun {
  return {
    id: row.id,
    jobId: row.jobId,
    status: row.status,
    startedAt: new Date(row.startedAt).toISOString(),
    completedAt: toIso(row.completedAt),
    result: orUndefined(row.result),
    finalResult: orUndefined(row.finalResult),
    executionLog: orUndefined(row.executionLog),
    toolCalls: orUndefined(row.toolCalls),
    error: orUndefined(row.error),
  }
}

export function toScheduledJobRunPayload(run: ScheduledJobRun) {
  return {
    jobId: run.jobId,
    status: run.status,
    // Required by the server, and a run always carries the time it began.
    startedAt: toEpoch(run.startedAt) ?? Date.now(),
    completedAt: toEpoch(run.completedAt),
    result: run.result,
    finalResult: run.finalResult,
    executionLog: run.executionLog,
    toolCalls: run.toolCalls,
    error: run.error,
  }
}
