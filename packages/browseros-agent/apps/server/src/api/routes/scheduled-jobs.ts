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

/**
 * Timestamps arrive as epoch numbers. The extension holds ISO strings today,
 * so the conversion belongs on its side of this boundary, keeping the database
 * consistent with the other tables here.
 */
const UpsertJobSchema = z.object({
  profileId: z.string().nullish(),
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

/** Bulk one-time import. Insert-if-absent, for the reason on the provider route. */
const ImportJobsSchema = z.object({
  jobs: z.array(UpsertJobSchema.extend({ id: z.string().min(1) })),
})

export function createScheduledJobRoutes(
  options: { store?: ScheduledJobStore } = {},
) {
  const store = options.store ?? dbScheduledJobStore

  return new Hono<Env>()
    .get('/', async (c) => c.json({ jobs: await store.list() }))
    .post('/import', zValidator('json', ImportJobsSchema), async (c) => {
      const imported: string[] = []
      const skipped: string[] = []
      for (const job of c.req.valid('json').jobs) {
        const saved = await store.insertIfAbsent(job)
        ;(saved ? imported : skipped).push(job.id)
      }
      return c.json({ imported, skipped })
    })
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
