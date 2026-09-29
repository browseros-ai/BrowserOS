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

// Cadence bounds mirror the create-task form so imports and direct/programmatic
// API writes cannot persist a schedule the extension alarm builder later chokes
// on (apps/app/lib/schedules/createAlarmFromJob.ts): daily splits scheduleTime
// on ":" into hours/minutes, and hourly/minutes hand scheduleInterval straight to
// chrome.alarms, so a missing or malformed value yields no alarm at all.
const HH_MM_24H = /^([01]\d|2[0-3]):[0-5]\d$/
const MIN_INTERVAL = 1
const MAX_INTERVAL = 60

/**
 * Timestamps arrive as epoch numbers. The extension holds ISO strings today,
 * so the conversion belongs on its side of this boundary, keeping the database
 * consistent with the other tables here.
 */
const UpsertJobSchema = z
  .object({
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
  .superRefine((job, ctx) => {
    if (job.scheduleType === 'daily') {
      if (!job.scheduleTime || !HH_MM_24H.test(job.scheduleTime)) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'daily jobs require scheduleTime as HH:MM (24-hour)',
          path: ['scheduleTime'],
        })
      }
      return
    }

    const interval = job.scheduleInterval
    if (
      interval == null ||
      !Number.isInteger(interval) ||
      interval < MIN_INTERVAL ||
      interval > MAX_INTERVAL
    ) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: `${job.scheduleType} jobs require an integer scheduleInterval between ${MIN_INTERVAL} and ${MAX_INTERVAL}`,
        path: ['scheduleInterval'],
      })
    }
  })

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
        const job = await store.upsert({
          ...c.req.valid('json'),
          id: c.req.valid('param').jobId,
        })
        return c.json({ job })
      },
    )
    .delete('/:jobId', zValidator('param', IdParamSchema), async (c) => {
      const deleted = await store.remove(c.req.valid('param').jobId)
      if (!deleted) return c.json({ error: 'Unknown scheduled job' }, 404)
      return c.json({ success: true })
    })
}
