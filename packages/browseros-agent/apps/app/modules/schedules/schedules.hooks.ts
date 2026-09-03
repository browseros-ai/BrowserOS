import { useMutation, useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { createQuery } from 'react-query-kit'
import { sendScheduleMessage } from '@/lib/messaging/schedules/scheduleMessages'
import { createAlarmFromJob } from '@/lib/schedules/createAlarmFromJob'
import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'
import {
  deleteScheduledJob,
  listScheduledJobRuns,
  listScheduledJobs,
  putScheduledJob,
} from './schedules.api'
import { watchScheduleRevision } from './schedules.revision'

const getAlarmName = (jobId: string) => `scheduled-job-${jobId}`

export const useScheduledJobsQuery = createQuery<ScheduledJob[]>({
  queryKey: ['scheduled-jobs'],
  fetcher: listScheduledJobs,
})

export const useScheduledJobRunsQuery = createQuery<ScheduledJobRun[]>({
  queryKey: ['scheduled-job-runs'],
  fetcher: listScheduledJobRuns,
})

/**
 * Keeps this view current with writes made in the background.
 *
 * The alarm runner records runs from a different context, which extension
 * storage used to surface through `watch`. The rows live on the server now, so
 * the background bumps a revision instead and every mounted view refetches.
 */
function useScheduleRevision(): void {
  const queryClient = useQueryClient()

  useEffect(
    () =>
      watchScheduleRevision(() => {
        queryClient.invalidateQueries({
          queryKey: useScheduledJobsQuery.getKey(),
        })
        queryClient.invalidateQueries({
          queryKey: useScheduledJobRunsQuery.getKey(),
        })
      }),
    [queryClient],
  )
}

export interface UseScheduledJobsReturn {
  jobs: ScheduledJob[]
  /** The server could not be reached, as opposed to reporting no jobs. */
  isUnavailable: boolean
  addJob: (
    job: Omit<ScheduledJob, 'id' | 'createdAt' | 'updatedAt'>,
  ) => Promise<void>
  removeJob: (id: string) => Promise<void>
  editJob: (
    id: string,
    updates: Omit<ScheduledJob, 'id' | 'createdAt' | 'updatedAt'>,
  ) => Promise<void>
  toggleJob: (id: string, enabled: boolean) => Promise<void>
  runJob: (id: string) => Promise<unknown>
}

export function useScheduledJobs(): UseScheduledJobsReturn {
  const queryClient = useQueryClient()
  const jobsQuery = useScheduledJobsQuery()
  useScheduleRevision()

  const jobs = jobsQuery.data ?? []
  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: useScheduledJobsQuery.getKey() })

  const saveMutation = useMutation({
    mutationFn: async (job: ScheduledJob) => {
      await putScheduledJob(job)
      // The alarm is the thing that actually makes a schedule fire, so it is
      // rebuilt from the saved job rather than the requested one.
      await chrome.alarms.clear(getAlarmName(job.id))
      if (job.enabled) await createAlarmFromJob(job)
    },
    onSuccess: invalidate,
  })

  const removeMutation = useMutation({
    mutationFn: async (id: string) => {
      await chrome.alarms.clear(getAlarmName(id))
      // Runs are removed with the job by the cascade on the row.
      await deleteScheduledJob(id)
    },
    onSuccess: () => {
      invalidate()
      queryClient.invalidateQueries({
        queryKey: useScheduledJobRunsQuery.getKey(),
      })
    },
  })

  const save = async (job: ScheduledJob) => {
    await saveMutation.mutateAsync(job)
  }

  return {
    jobs,
    isUnavailable: jobsQuery.isError,
    addJob: async (job) => {
      const now = new Date().toISOString()
      await save({
        ...job,
        id: crypto.randomUUID(),
        createdAt: now,
        updatedAt: now,
      })
    },
    removeJob: async (id) => {
      await removeMutation.mutateAsync(id)
    },
    editJob: async (id, updates) => {
      const existing = jobs.find((job) => job.id === id)
      if (!existing) return
      await save({
        ...updates,
        id,
        createdAt: existing.createdAt,
        updatedAt: new Date().toISOString(),
      })
    },
    toggleJob: async (id, enabled) => {
      const existing = jobs.find((job) => job.id === id)
      if (!existing) return
      await save({ ...existing, enabled, updatedAt: new Date().toISOString() })
    },
    runJob: (id) => sendScheduleMessage('runScheduledJob', { jobId: id }),
  }
}

export interface UseScheduledJobRunsReturn {
  jobRuns: ScheduledJobRun[]
  isUnavailable: boolean
  cancelJobRun: (runId: string) => Promise<unknown>
}

export function useScheduledJobRuns(): UseScheduledJobRunsReturn {
  const runsQuery = useScheduledJobRunsQuery()
  useScheduleRevision()

  return {
    jobRuns: runsQuery.data ?? [],
    isUnavailable: runsQuery.isError,
    cancelJobRun: (runId) =>
      sendScheduleMessage('cancelScheduledJobRun', { runId }),
  }
}
