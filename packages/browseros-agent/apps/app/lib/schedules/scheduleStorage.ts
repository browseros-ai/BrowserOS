import { storage } from '@wxt-dev/storage'
import type { ScheduledJob, ScheduledJobRun } from './scheduleTypes'

/**
 * Legacy extension storage for scheduled jobs and their runs.
 *
 * The server owns both now. These items remain only as the source the one-time
 * import reads, and nothing writes them any more.
 */

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
