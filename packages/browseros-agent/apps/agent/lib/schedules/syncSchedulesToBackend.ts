import { isEqual, omit } from 'es-toolkit'
import { GetProfileIdByUserIdDocument } from '@/lib/conversations/graphql/uploadConversationDocument'
import { execute } from '@/lib/graphql/execute'
import { sentry } from '@/lib/sentry/sentry'
import { createAlarmFromJob } from './createAlarmFromJob'
import {
  CreateScheduledJobDocument,
  GetScheduledJobsByProfileIdDocument,
  UpdateScheduledJobDocument,
} from './graphql/syncSchedulesDocument'
import { scheduledJobStorage } from './scheduleStorage'
import type { ScheduledJob } from './scheduleTypes'

type RemoteScheduledJob = {
  rowId: string
  name: string
  query: string
  scheduleType: string
  scheduleTime: string | null
  scheduleInterval: number | null
  enabled: boolean
  llmProviderId: string | null
  createdAt: string
  updatedAt: string
  lastRunAt: string | null
}

const IGNORED_FIELDS = ['id', 'createdAt', 'lastRunAt'] as const

function toComparable(job: ScheduledJob) {
  const data = omit(job, IGNORED_FIELDS)
  return {
    ...data,
    scheduleTime: data.scheduleTime ?? null,
    scheduleInterval: data.scheduleInterval ?? null,
    providerId: data.providerId ?? null,
  }
}

function remoteToComparable(job: RemoteScheduledJob) {
  return {
    name: job.name,
    query: job.query,
    scheduleType: job.scheduleType as ScheduledJob['scheduleType'],
    scheduleTime: job.scheduleTime,
    scheduleInterval: job.scheduleInterval,
    enabled: job.enabled,
    providerId: job.llmProviderId,
  }
}

function normalizeTimestamp(ts: string): string {
  return ts.endsWith('Z') ? ts : `${ts}Z`
}

function remoteToLocal(remote: RemoteScheduledJob): ScheduledJob {
  return {
    id: remote.rowId,
    name: remote.name,
    query: remote.query,
    scheduleType: remote.scheduleType as ScheduledJob['scheduleType'],
    scheduleTime: remote.scheduleTime ?? undefined,
    scheduleInterval: remote.scheduleInterval ?? undefined,
    enabled: remote.enabled,
    providerId: remote.llmProviderId ?? undefined,
    createdAt: normalizeTimestamp(remote.createdAt),
    updatedAt: normalizeTimestamp(remote.updatedAt),
    lastRunAt: remote.lastRunAt
      ? normalizeTimestamp(remote.lastRunAt)
      : undefined,
  }
}

function getLocalUpdatedAt(job: ScheduledJob): Date {
  return new Date(job.updatedAt || job.createdAt)
}

function getRemoteUpdatedAt(remote: RemoteScheduledJob): Date {
  return new Date(normalizeTimestamp(remote.updatedAt))
}

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: TODO(dani) refactor to reduce complexity
export async function syncSchedulesToBackend(
  localJobs: ScheduledJob[],
  userId: string,
): Promise<void> {
  const profileResult = await execute(GetProfileIdByUserIdDocument, { userId })
  const profileId = profileResult.profileByUserId?.rowId
  if (!profileId) return

  const remoteResult = await execute(GetScheduledJobsByProfileIdDocument, {
    profileId,
  })

  const remoteJobs = new Map<string, RemoteScheduledJob>()
  for (const node of remoteResult.scheduledJobs?.nodes ?? []) {
    if (node) {
      remoteJobs.set(node.rowId, node as RemoteScheduledJob)
    }
  }

  const localJobsMap = new Map(localJobs.map((j) => [j.id, j]))
  const jobsToAddLocally: ScheduledJob[] = []
  const jobsToUpdateLocally: ScheduledJob[] = []

  for (const [rowId, remote] of remoteJobs) {
    const localJob = localJobsMap.get(rowId)
    if (!localJob) {
      jobsToAddLocally.push(remoteToLocal(remote))
    } else {
      const localTime = getLocalUpdatedAt(localJob)
      const remoteTime = getRemoteUpdatedAt(remote)

      if (remoteTime > localTime) {
        jobsToUpdateLocally.push(remoteToLocal(remote))
      }
    }
  }

  if (jobsToAddLocally.length > 0 || jobsToUpdateLocally.length > 0) {
    const currentJobs = (await scheduledJobStorage.getValue()) ?? []
    const existingIds = new Set(currentJobs.map((j) => j.id))

    const newJobs = jobsToAddLocally.filter((j) => !existingIds.has(j.id))

    const mergedJobs = currentJobs.map((j) => {
      const updated = jobsToUpdateLocally.find((u) => u.id === j.id)
      return updated ?? j
    })

    if (newJobs.length > 0 || jobsToUpdateLocally.length > 0) {
      await scheduledJobStorage.setValue([...mergedJobs, ...newJobs])

      for (const job of [...newJobs, ...jobsToUpdateLocally]) {
        try {
          const alarmName = `scheduled-job-${job.id}`
          await chrome.alarms.clear(alarmName)
          if (job.enabled) {
            await createAlarmFromJob(job)
          }
        } catch {
          // Alarm operations may fail in non-background context
        }
      }
    }
  }

  for (const job of localJobs) {
    try {
      const remote = remoteJobs.get(job.id)

      if (remote) {
        const localTime = getLocalUpdatedAt(job)
        const remoteTime = getRemoteUpdatedAt(remote)

        if (remoteTime >= localTime) continue

        if (isEqual(toComparable(job), remoteToComparable(remote))) continue

        await execute(UpdateScheduledJobDocument, {
          input: {
            rowId: job.id,
            patch: {
              name: job.name,
              query: job.query,
              scheduleType: job.scheduleType,
              scheduleTime: job.scheduleTime ?? null,
              scheduleInterval: job.scheduleInterval ?? null,
              enabled: job.enabled,
              llmProviderId: job.providerId ?? null,
              lastRunAt: job.lastRunAt
                ? new Date(job.lastRunAt).toISOString()
                : null,
              updatedAt: job.updatedAt || new Date().toISOString(),
            },
          },
        })
      } else {
        await execute(CreateScheduledJobDocument, {
          input: {
            scheduledJob: {
              rowId: job.id,
              profileId,
              name: job.name,
              query: job.query,
              scheduleType: job.scheduleType,
              scheduleTime: job.scheduleTime ?? null,
              scheduleInterval: job.scheduleInterval ?? null,
              enabled: job.enabled,
              llmProviderId: job.providerId ?? null,
              createdAt: new Date(job.createdAt).toISOString(),
              updatedAt: job.updatedAt || new Date().toISOString(),
              lastRunAt: job.lastRunAt
                ? new Date(job.lastRunAt).toISOString()
                : null,
            },
          },
        })
      }
    } catch (error) {
      sentry.captureException(error, {
        extra: {
          jobId: job.id,
          jobName: job.name,
        },
      })
    }
  }
}
