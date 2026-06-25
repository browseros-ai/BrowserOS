import { storage } from '@wxt-dev/storage'
import { useEffect, useState } from 'react'
import { sendScheduleMessage } from '@/lib/messaging/schedules/scheduleMessages'
import { createAlarmFromJob } from './createAlarmFromJob'
import type { ScheduledJob, ScheduledJobRun } from './scheduleTypes'

const getAlarmName = (jobId: string) => `scheduled-job-${jobId}`

export const scheduledJobStorage = storage.defineItem<ScheduledJob[]>(
  'local:scheduledJobs',
  {
    fallback: [],
  },
)

export const scheduledJobRunStorage = storage.defineItem<ScheduledJobRun[]>(
  'local:scheduledJobRuns',
  {
    fallback: [],
  },
)

export const pendingDeletionStorage = storage.defineItem<string[]>(
  'local:scheduledJobsPendingDeletion',
  {
    fallback: [],
  },
)

export function useScheduledJobs() {
  const [jobs, setJobs] = useState<ScheduledJob[]>([])

  useEffect(() => {
    scheduledJobStorage.getValue().then(setJobs)
    const unwatch = scheduledJobStorage.watch((newValue) => {
      setJobs(newValue ?? [])
    })
    return unwatch
  }, [])

  const addJob = async (
    job: Omit<ScheduledJob, 'id' | 'createdAt' | 'updatedAt'>,
  ) => {
    const now = new Date().toISOString()
    const newJob: ScheduledJob = {
      id: crypto.randomUUID(),
      createdAt: now,
      updatedAt: now,
      ...job,
    }
    const current = (await scheduledJobStorage.getValue()) ?? []
    await scheduledJobStorage.setValue([...current, newJob])

    if (newJob.enabled) {
      await createAlarmFromJob(newJob)
    }
  }

  const removeJob = async (id: string) => {
    await chrome.alarms.clear(getAlarmName(id))

    const pending = (await pendingDeletionStorage.getValue()) ?? []
    if (!pending.includes(id)) {
      await pendingDeletionStorage.setValue([...pending, id])
    }

    const currentJobs = (await scheduledJobStorage.getValue()) ?? []
    await scheduledJobStorage.setValue(currentJobs.filter((j) => j.id !== id))

    const currentRuns = (await scheduledJobRunStorage.getValue()) ?? []
    await scheduledJobRunStorage.setValue(
      currentRuns.filter((r) => r.jobId !== id),
    )
  }

  const toggleJob = async (id: string, enabled: boolean) => {
    const current = (await scheduledJobStorage.getValue()) ?? []
    const job = current.find((j) => j.id === id)
    if (!job) return

    const updatedAt = new Date().toISOString()
    await scheduledJobStorage.setValue(
      current.map((j) => (j.id === id ? { ...j, enabled, updatedAt } : j)),
    )

    if (enabled) {
      await createAlarmFromJob({ ...job, enabled })
    } else {
      await chrome.alarms.clear(getAlarmName(id))
    }
  }

  const editJob = async (
    id: string,
    updates: Omit<ScheduledJob, 'id' | 'createdAt' | 'updatedAt'>,
  ) => {
    const current = (await scheduledJobStorage.getValue()) ?? []
    const existingJob = current.find((j) => j.id === id)
    if (!existingJob) return

    const updatedJob: ScheduledJob = {
      id,
      createdAt: existingJob.createdAt,
      updatedAt: new Date().toISOString(),
      ...updates,
    }
    await scheduledJobStorage.setValue(
      current.map((j) => (j.id === id ? updatedJob : j)),
    )

    await chrome.alarms.clear(getAlarmName(id))
    if (updatedJob.enabled) {
      await createAlarmFromJob(updatedJob)
    }
  }

  const runJob = async (id: string) => {
    return sendScheduleMessage('runScheduledJob', { jobId: id })
  }

  return { jobs, addJob, removeJob, editJob, toggleJob, runJob }
}

export function useScheduledJobRuns() {
  const [jobRuns, setJobRuns] = useState<ScheduledJobRun[]>([])

  useEffect(() => {
    scheduledJobRunStorage.getValue().then(setJobRuns)
    const unwatch = scheduledJobRunStorage.watch((newValue) => {
      setJobRuns(newValue ?? [])
    })
    return unwatch
  }, [])

  const addJobRun = async (jobRun: ScheduledJobRun) => {
    const current = (await scheduledJobRunStorage.getValue()) ?? []
    await scheduledJobRunStorage.setValue([...current, jobRun])
  }

  const removeJobRun = async (id: string) => {
    const current = (await scheduledJobRunStorage.getValue()) ?? []
    await scheduledJobRunStorage.setValue(current.filter((r) => r.id !== id))
  }

  const editJobRun = async (
    id: string,
    updates: Partial<Omit<ScheduledJobRun, 'id'>>,
  ) => {
    const current = (await scheduledJobRunStorage.getValue()) ?? []
    await scheduledJobRunStorage.setValue(
      current.map((r) => (r.id === id ? { ...r, ...updates } : r)),
    )
  }

  const cancelJobRun = async (runId: string) => {
    return sendScheduleMessage('cancelScheduledJobRun', { runId })
  }

  return { jobRuns, addJobRun, removeJobRun, editJobRun, cancelJobRun }
}
