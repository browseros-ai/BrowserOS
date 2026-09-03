import type {
  ScheduledJobRoutes,
  ScheduledJobRunRoutes,
} from '@browseros/server'
import { hc } from 'hono/client'
import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'
import { resolveAgentServerUrlWithRetry } from '@/modules/browseros/agent-server-url.helpers'
import {
  type ScheduledJobRow,
  type ScheduledJobRunRow,
  toScheduledJob,
  toScheduledJobPayload,
  toScheduledJobRun,
  toScheduledJobRunPayload,
} from './schedules.helpers'
import { bumpScheduleRevision } from './schedules.revision'

async function jobsClient() {
  const baseUrl = await resolveAgentServerUrlWithRetry()
  return hc<ScheduledJobRoutes>(`${baseUrl}/scheduled-jobs`)
}

async function runsClient() {
  const baseUrl = await resolveAgentServerUrlWithRetry()
  return hc<ScheduledJobRunRoutes>(`${baseUrl}/scheduled-job-runs`)
}

export async function listScheduledJobs(): Promise<ScheduledJob[]> {
  const client = await jobsClient()
  const response = await client.index.$get()
  if (!response.ok) {
    throw new Error(`Failed to load scheduled jobs (${response.status})`)
  }
  const { jobs } = await response.json()
  return (jobs as ScheduledJobRow[]).map(toScheduledJob)
}

export async function putScheduledJob(job: ScheduledJob): Promise<void> {
  const client = await jobsClient()
  const response = await client[':jobId'].$put({
    param: { jobId: job.id },
    json: toScheduledJobPayload(job),
  })
  if (!response.ok) {
    throw new Error(`Failed to save scheduled job (${response.status})`)
  }
  await bumpScheduleRevision()
}

export async function deleteScheduledJob(jobId: string): Promise<void> {
  const client = await jobsClient()
  const response = await client[':jobId'].$delete({ param: { jobId } })
  if (!response.ok && response.status !== 404) {
    throw new Error(`Failed to delete scheduled job (${response.status})`)
  }
  await bumpScheduleRevision()
}

export async function listScheduledJobRuns(): Promise<ScheduledJobRun[]> {
  const client = await runsClient()
  const response = await client.index.$get()
  if (!response.ok) {
    throw new Error(`Failed to load run history (${response.status})`)
  }
  const { runs } = await response.json()
  return (runs as ScheduledJobRunRow[]).map(toScheduledJobRun)
}

export async function putScheduledJobRun(run: ScheduledJobRun): Promise<void> {
  const client = await runsClient()
  const response = await client[':runId'].$put({
    param: { runId: run.id },
    json: toScheduledJobRunPayload(run),
  })
  if (!response.ok) {
    throw new Error(`Failed to save run (${response.status})`)
  }
  await bumpScheduleRevision()
}

/**
 * Jobs for callers outside React, returning null when the server could not be
 * reached. The alarm runner uses this to tell "no jobs are due" apart from
 * "the list did not load", which otherwise look identical and would silently
 * skip every scheduled task.
 */
export async function listScheduledJobsOrNull(): Promise<
  ScheduledJob[] | null
> {
  try {
    return await listScheduledJobs()
  } catch {
    return null
  }
}

export async function listScheduledJobRunsOrNull(): Promise<
  ScheduledJobRun[] | null
> {
  try {
    return await listScheduledJobRuns()
  } catch {
    return null
  }
}

/** One-time import of run history from extension storage. */
export async function importScheduledJobRuns(
  runs: ScheduledJobRun[],
): Promise<void> {
  const client = await runsClient()
  const response = await client.import.$post({
    json: {
      runs: runs.map((run) => ({
        ...toScheduledJobRunPayload(run),
        id: run.id,
      })),
    },
  })
  if (!response.ok) {
    throw new Error(`Failed to import run history (${response.status})`)
  }
}
