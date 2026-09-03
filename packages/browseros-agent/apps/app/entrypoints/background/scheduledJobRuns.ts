import { onScheduleMessage } from '@/lib/messaging/schedules/scheduleMessages'
import { createAlarmFromJob } from '@/lib/schedules/createAlarmFromJob'
import { getChatServerResponse } from '@/lib/schedules/getChatServerResponse'
import type { ScheduledJobRun } from '@/lib/schedules/scheduleTypes'
import {
  listScheduledJobRunsOrNull,
  listScheduledJobsOrNull,
  putScheduledJob,
  putScheduledJobRun,
} from '@/modules/schedules/schedules.api'
import { applyLastRunAt } from '@/modules/schedules/schedules.helpers'

const STALE_TIMEOUT_MS = 10 * 60 * 1000 // 10 minutes
const TWENTY_FOUR_HOURS_MS = 24 * 60 * 60 * 1000

const runAbortControllers = new Map<string, AbortController>()

export const scheduledJobRuns = async () => {
  // Every read below distinguishes an unreachable server from an empty list.
  // Treating the two alike would look like "nothing is scheduled": alarms would
  // not be rebuilt on startup and schedules would quietly stop firing, with no
  // failed run to show for it. Skipping the pass instead leaves the next
  // startup to retry.
  const cleanupStaleJobRuns = async () => {
    const current = await listScheduledJobRunsOrNull()
    if (current === null) return
    const now = Date.now()

    const stale = current.filter(
      (run) =>
        run.status === 'running' &&
        now - new Date(run.startedAt).getTime() > STALE_TIMEOUT_MS,
    )

    for (const run of stale) {
      await putScheduledJobRun({
        ...run,
        status: 'failed',
        completedAt: new Date().toISOString(),
        result: 'Job timed out!',
      })
    }
  }

  const syncAlarmState = async () => {
    const loaded = await listScheduledJobsOrNull()
    if (loaded === null) return
    const jobs = loaded.filter((each) => each.enabled)

    for (let i = 0; i < jobs.length; i++) {
      const job = jobs[i]
      const alarmName = `scheduled-job-${job.id}`
      const existingAlarm = await chrome.alarms.get(alarmName)

      if (!existingAlarm) {
        await createAlarmFromJob(job)
      }
    }
  }

  const createJobRun = async (
    jobId: string,
    status: ScheduledJobRun['status'],
  ): Promise<ScheduledJobRun> => {
    // Trimming to the per-job cap happens on the server now, so creating a run
    // no longer has to rewrite the job's whole history to stay bounded.
    const jobRun: ScheduledJobRun = {
      id: crypto.randomUUID(),
      jobId,
      startedAt: new Date().toISOString(),
      status,
    }

    await putScheduledJobRun(jobRun)
    return jobRun
  }

  // Takes the run rather than its id: the caller already holds it, and merging
  // locally avoids re-reading a list to update one row.
  const updateJobRun = async (
    run: ScheduledJobRun,
    updates: Partial<Omit<ScheduledJobRun, 'id' | 'jobId' | 'startedAt'>>,
  ) => {
    await putScheduledJobRun({ ...run, ...updates })
  }

  // Takes an id, not the job: a snapshot captured before the run would be
  // minutes stale by the time this writes, and putting it back would revert any
  // edit made while the run was going.
  const updateJobLastRunAt = async (jobId: string) => {
    const jobs = await listScheduledJobsOrNull()
    if (jobs === null) return

    const updated = applyLastRunAt(jobs, jobId, new Date().toISOString())
    if (updated) await putScheduledJob(updated)
  }

  const executeScheduledJob = async (jobId: string): Promise<void> => {
    const jobs = await listScheduledJobsOrNull()
    if (jobs === null) {
      throw new Error('Cannot reach the BrowserOS server to load the job')
    }

    const job = jobs.find((each) => each.id === jobId)
    if (!job) {
      throw new Error(`Job not found: ${jobId}`)
    }

    const jobRun = await createJobRun(jobId, 'running')
    const abortController = new AbortController()
    runAbortControllers.set(jobRun.id, abortController)

    try {
      const response = await getChatServerResponse({
        message: job.query,
        signal: abortController.signal,
        providerId: job.providerId,
      })

      await updateJobRun(jobRun, {
        status: 'completed',
        completedAt: new Date().toISOString(),
        result: response.text,
        finalResult: response.finalResult,
        executionLog: response.executionLog,
        toolCalls: response.toolCalls,
      })
    } catch (e) {
      const isCancelled = abortController.signal.aborted
      const errorMessage = isCancelled
        ? 'Cancelled by user'
        : e instanceof Error
          ? e.message
          : String(e)
      await updateJobRun(jobRun, {
        status: 'failed',
        completedAt: new Date().toISOString(),
        result: errorMessage,
        error: errorMessage,
      })
    } finally {
      runAbortControllers.delete(jobRun.id)
      await updateJobLastRunAt(jobId)
    }
  }

  let runningMissedJobs = false

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: TODO(dani) refactor to reduce complexity
  const runMissedJobs = async () => {
    if (runningMissedJobs) return
    runningMissedJobs = true

    try {
      const loadedJobs = await listScheduledJobsOrNull()
      const runs = await listScheduledJobRunsOrNull()
      if (loadedJobs === null || runs === null) return

      const jobs = loadedJobs.filter((j) => j.enabled)
      const now = Date.now()
      const cutoff = now - TWENTY_FOUR_HOURS_MS

      for (const job of jobs) {
        const hasRecentRun = runs.some(
          (r) => r.jobId === job.id && new Date(r.startedAt).getTime() > cutoff,
        )
        if (hasRecentRun) continue

        const hasRunningRun = runs.some(
          (r) => r.jobId === job.id && r.status === 'running',
        )
        if (hasRunningRun) continue

        if (job.scheduleType === 'daily' && job.scheduleTime) {
          const [hours, minutes] = job.scheduleTime.split(':').map(Number)
          const scheduledToday = new Date()
          scheduledToday.setHours(hours, minutes, 0, 0)
          if (now < scheduledToday.getTime()) continue
        }

        if (
          (job.scheduleType === 'hourly' || job.scheduleType === 'minutes') &&
          job.scheduleInterval
        ) {
          const intervalMs =
            job.scheduleType === 'hourly'
              ? job.scheduleInterval * 60 * 60 * 1000
              : job.scheduleInterval * 60 * 1000
          const createdAt = new Date(job.createdAt).getTime()
          if (now - createdAt < intervalMs) continue
        }

        await executeScheduledJob(job.id)
      }
    } finally {
      runningMissedJobs = false
    }
  }

  chrome.alarms.onAlarm.addListener(async (alarm) => {
    if (!alarm.name.startsWith('scheduled-job-')) return
    const jobId = alarm.name.replace('scheduled-job-', '')
    await executeScheduledJob(jobId)
  })

  onScheduleMessage('runScheduledJob', async ({ data }) => {
    try {
      await executeScheduledJob(data.jobId)
      return { success: true }
    } catch (e) {
      return {
        success: false,
        error: e instanceof Error ? e.message : String(e),
      }
    }
  })

  onScheduleMessage('cancelScheduledJobRun', async ({ data }) => {
    const controller = runAbortControllers.get(data.runId)
    if (!controller) {
      return { success: false, error: 'Run not found or already completed' }
    }
    controller.abort()
    return { success: true }
  })

  chrome.runtime.onStartup.addListener(async () => {
    await cleanupStaleJobRuns()
    await syncAlarmState()
    await runMissedJobs()
  })

  chrome.runtime.onInstalled.addListener(async () => {
    await cleanupStaleJobRuns()
    await syncAlarmState()
    await runMissedJobs()
  })
}
