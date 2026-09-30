/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import {
  dbScheduledJobStore,
  type ScheduledJobStore,
} from '../../lib/schedules/schedule-store'
import type { Env } from '../types'

const IdParamSchema = z.object({ jobId: z.string().min(1) })

const HH_MM_24H = /^([01]\d|2[0-3]):[0-5]\d$/
const MIN_INTERVAL = 1
const MAX_INTERVAL = 60

/**
 * Timestamps arrive as epoch numbers. The extension holds ISO strings today,
 * so the conversion belongs on its side of this boundary, keeping the database
 * consistent with the other tables here.
 */
const UpsertJobSchema = z.object({
  name: z.string().min(1),
  query: z.string().min(1),
  scheduleType: z.enum(['daily', 'hourly', 'minutes']),
  scheduleTime: z.string().nullish(),
  scheduleInterval: z.number().nullish(),
  enabled: z.boolean().optional(),
  providerId: z.string().nullish(),
  lastRunAt: z.number().nullish(),
  createdAt: z.number().optional(),
})

type UpsertJob = z.infer<typeof UpsertJobSchema>

type Cadence = Pick<
  UpsertJob,
  'scheduleType' | 'scheduleTime' | 'scheduleInterval'
>

/**
 * The cadence the extension alarm builder can actually use
 * (apps/app/lib/schedules/createAlarmFromJob.ts): daily splits scheduleTime on
 * ":" into hours/minutes so it needs an HH:MM (24-hour) time, and hourly/minutes
 * hand scheduleInterval straight to chrome.alarms so it needs a whole-number
 * interval in [1, 60]. Returns an error message when the cadence is unusable, or
 * null when it is fine. Mirrors the create-task form so imports and direct or
 * programmatic writes cannot persist a schedule that never produces an alarm.
 */
function cadenceProblem(job: Cadence): string | null {
  if (job.scheduleType === 'daily') {
    if (!job.scheduleTime || !HH_MM_24H.test(job.scheduleTime)) {
      return 'daily jobs require scheduleTime as HH:MM (24-hour)'
    }
    return null
  }

  const interval = job.scheduleInterval
  if (
    interval == null ||
    !Number.isInteger(interval) ||
    interval < MIN_INTERVAL ||
    interval > MAX_INTERVAL
  ) {
    return `${job.scheduleType} jobs require an integer scheduleInterval between ${MIN_INTERVAL} and ${MAX_INTERVAL}`
  }
  return null
}

/**
 * True when the write introduces or changes the cadence. Maintenance writes that
 * keep an existing cadence, recording lastRunAt or toggling enabled, leave it
 * unchanged, so they are not re-gated and a job stored before this validation
 * never becomes unwritable because of an older invalid cadence.
 */
function cadenceChanged(existing: Cadence | null, next: Cadence): boolean {
  if (!existing) return true
  return (
    existing.scheduleType !== next.scheduleType ||
    (existing.scheduleTime ?? null) !== (next.scheduleTime ?? null) ||
    (existing.scheduleInterval ?? null) !== (next.scheduleInterval ?? null)
  )
}

export function createScheduledJobRoutes(
  options: { store?: ScheduledJobStore } = {},
) {
  const store = options.store ?? dbScheduledJobStore

  return new Hono<Env>()
    .get('/', async (c) => c.json({ jobs: await store.list() }))
    .get('/:jobId', zValidator('param', IdParamSchema), async (c) => {
      const job = await store.get(c.req.valid('param').jobId)
      if (!job) return c.json({ error: 'Unknown scheduled job' }, 404)
      return c.json({ job })
    })
    .put(
      '/:jobId',
      zValidator('param', IdParamSchema),
      zValidator('json', UpsertJobSchema),
      async (c) => {
        const jobId = c.req.valid('param').jobId
        const input = c.req.valid('json')
        const existing = await store.get(jobId)
        if (cadenceChanged(existing, input)) {
          const problem = cadenceProblem(input)
          if (problem) return c.json({ error: problem }, 400)
        }
        const job = await store.upsert({ ...input, id: jobId })
        return c.json({ job })
      },
    )
    .delete('/:jobId', zValidator('param', IdParamSchema), async (c) => {
      const deleted = await store.remove(c.req.valid('param').jobId)
      if (!deleted) return c.json({ error: 'Unknown scheduled job' }, 404)
      return c.json({ success: true })
    })
}
