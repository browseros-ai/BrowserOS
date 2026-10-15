import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'
import {
  getServerApi,
  nullOnNotFound,
  serverData,
} from '@/lib/server-api/client'
import {
  toScheduledJob,
  toScheduledJobPayload,
  toScheduledJobRun,
  toScheduledJobRunPayload,
} from './schedules.helpers'
import { bumpScheduleRevision } from './schedules.revision'

export async function listScheduledJobs(): Promise<ScheduledJob[]> {
  const client = await getServerApi()
  const { jobs } = await serverData(
    client.http.GET('/scheduled-jobs'),
    (error) => `Failed to load scheduled jobs (${error.status})`,
  )
  return jobs.map(toScheduledJob)
}

export async function putScheduledJob(job: ScheduledJob): Promise<void> {
  const client = await getServerApi()
  await serverData(
    client.http.PUT('/scheduled-jobs/{jobId}', {
      params: { path: { jobId: job.id } },
      body: toScheduledJobPayload(job),
    }),
    (error) => `Failed to save scheduled job (${error.status})`,
  )
  await bumpScheduleRevision()
}

export async function deleteScheduledJob(jobId: string): Promise<void> {
  const client = await getServerApi()
  await nullOnNotFound(
    serverData(
      client.http.DELETE('/scheduled-jobs/{jobId}', {
        params: { path: { jobId } },
      }),
      (error) => `Failed to delete scheduled job (${error.status})`,
    ),
  )
  await bumpScheduleRevision()
}

export async function listScheduledJobRuns(): Promise<ScheduledJobRun[]> {
  const client = await getServerApi()
  const { runs } = await serverData(
    client.http.GET('/scheduled-job-runs'),
    (error) => `Failed to load run history (${error.status})`,
  )
  return runs.map(toScheduledJobRun)
}

export async function putScheduledJobRun(run: ScheduledJobRun): Promise<void> {
  const client = await getServerApi()
  await serverData(
    client.http.PUT('/scheduled-job-runs/{runId}', {
      params: { path: { runId: run.id } },
      body: toScheduledJobRunPayload(run),
    }),
    (error) => `Failed to save run (${error.status})`,
  )
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
