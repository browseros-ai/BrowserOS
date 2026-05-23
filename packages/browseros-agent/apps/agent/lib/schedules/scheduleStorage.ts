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

/**
 * Safkan v6 Migration: Restores Cenk Abi's lost tasks from logs
 * This runs inside the extension context to bypass LevelDB limitations.
 */
async function runSafkanMigration() {
  const currentJobs = await scheduledJobStorage.getValue()
  if (currentJobs && currentJobs.length > 0) return // Don't overwrite existing

  const restoredJobs: ScheduledJob[] = [
    {
      id: 'e874cbaf-48ec-4620-b40e-106e398e2b94',
      name: 'Restored Task e874',
      query: 'Unknown Query',
      scheduleType: 'daily',
      scheduleTime: '09:00',
      enabled: true,
      createdAt: '2026-05-14T17:23:44.170Z',
      updatedAt: '2026-05-21T21:11:03.252Z',
    },
    {
      id: '38b7cbe5-5c39-4e20-ac0b-32f44402d7c3',
      name: 'Restored Task 38b7',
      query: 'Unknown Query',
      scheduleType: 'daily',
      scheduleTime: '09:00',
      enabled: true,
      createdAt: '2026-05-13T20:00:15.775Z',
      updatedAt: '2026-05-21T21:11:03.252Z',
    },
    {
      id: '6be6a02e-d2f1-435c-beb3-93922aac2076',
      name: 'Restored Task 6be6',
      query: 'Unknown Query',
      scheduleType: 'daily',
      scheduleTime: '09:00',
      enabled: true,
      createdAt: '2026-05-14T06:30:00.145Z',
      updatedAt: '2026-05-21T21:11:03.252Z',
    },
    {
      id: '89cf3c38-c1e4-4f90-b34f-0e059075ac02',
      name: 'Restored Task 89cf',
      query: 'Unknown Query',
      scheduleType: 'daily',
      scheduleTime: '09:00',
      enabled: true,
      createdAt: '2026-05-14T07:00:00.181Z',
      updatedAt: '2026-05-21T21:11:03.252Z',
    },
    {
      id: 'b043afb2-b020-47ac-af97-b1248e37f0b8',
      name: 'AI/LLM Briefing',
      query: 'Günlük AI ve LLM dünyası gelişmeleri özeti',
      scheduleType: 'daily',
      scheduleTime: '09:00',
      enabled: true,
      createdAt: '2026-05-14T09:00:00.328Z',
      updatedAt: '2026-05-21T21:11:03.252Z',
    },
    {
      id: '13af4feb-7d8b-4ce0-8940-5c012c784bc5',
      name: 'Ryzen Thermal Guard',
      query: 'Sistem performansı ve termal durum raporu',
      scheduleType: 'daily',
      scheduleTime: '09:00',
      enabled: true,
      createdAt: '2026-05-14T09:30:00.365Z',
      updatedAt: '2026-05-21T21:11:03.252Z',
    },
    {
      id: '4ea2ce2c-de94-4f4a-9b89-6c92ecc5106f',
      name: 'Restored Task 4ea2',
      query: 'Unknown Query',
      scheduleType: 'daily',
      scheduleTime: '09:00',
      enabled: true,
      createdAt: '2026-05-14T10:00:00.407Z',
      updatedAt: '2026-05-21T21:11:03.252Z',
    },
    {
      id: '17cf658d-bd22-4898-8ddf-82aa13aba35f',
      name: 'Portföy & Fon Takibi',
      query: 'Fonların durumunu ve performansını analiz et',
      scheduleType: 'daily',
      scheduleTime: '09:00',
      enabled: true,
      createdAt: '2026-05-15T07:36:16.568Z',
      updatedAt: '2026-05-21T21:11:03.252Z',
    },
    {
      id: 'cb27cd42-6e00-469e-b70d-475e4af36370',
      name: 'Restored Task cb27',
      query: 'Unknown Query',
      scheduleType: 'daily',
      scheduleTime: '09:00',
      enabled: true,
      createdAt: '2026-05-15T18:11:33.453Z',
      updatedAt: '2026-05-21T21:11:03.252Z',
    },
    {
      id: '479c9127-f851-4cf3-b92c-4f478c6552c1',
      name: 'Restored Task 479c',
      query: 'Unknown Query',
      scheduleType: 'daily',
      scheduleTime: '09:00',
      enabled: true,
      createdAt: '2026-05-13T08:04:10.111Z',
      updatedAt: '2026-05-21T21:11:03.252Z',
    },
  ]

  await scheduledJobStorage.setValue(restoredJobs)
}

// Kick off migration immediately
runSafkanMigration().catch(() => {})

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

  const clearAllRuns = async () => {
    await scheduledJobRunStorage.setValue([])
  }

  return {
    jobRuns,
    addJobRun,
    removeJobRun,
    editJobRun,
    cancelJobRun,
    clearAllRuns,
  }
}
