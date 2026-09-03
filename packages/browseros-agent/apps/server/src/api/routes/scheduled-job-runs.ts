/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import {
  dbScheduledJobRunStore,
  type ScheduledJobRunStore,
} from '../../lib/schedules/run-store'
import type { Env } from '../types'

const IdParamSchema = z.object({ runId: z.string().min(1) })

const ToolCallSchema = z.object({
  id: z.string(),
  name: z.string(),
  input: z.unknown(),
  output: z.unknown().optional(),
  error: z.string().optional(),
  timestamp: z.string(),
})

/**
 * Timestamps arrive as epoch numbers. The extension holds ISO strings, so the
 * conversion belongs on its side of this boundary, keeping the database
 * consistent with the other tables here.
 */
const UpsertRunSchema = z.object({
  profileId: z.string().nullish(),
  jobId: z.string().min(1),
  status: z.enum(['running', 'completed', 'failed']),
  startedAt: z.number(),
  completedAt: z.number().nullish(),
  result: z.string().nullish(),
  finalResult: z.string().nullish(),
  executionLog: z.string().nullish(),
  toolCalls: z.array(ToolCallSchema).nullish(),
  error: z.string().nullish(),
  createdAt: z.number().optional(),
})

/** Bulk one-time import. Insert-if-absent, for the reason on the provider route. */
const ImportRunsSchema = z.object({
  runs: z.array(UpsertRunSchema.extend({ id: z.string().min(1) })),
})

export function createScheduledJobRunRoutes(
  options: { store?: ScheduledJobRunStore } = {},
) {
  const store = options.store ?? dbScheduledJobRunStore

  return new Hono<Env>()
    .get('/', async (c) => c.json({ runs: await store.list() }))
    .post('/import', zValidator('json', ImportRunsSchema), async (c) => {
      const imported: string[] = []
      const skipped: string[] = []
      for (const run of c.req.valid('json').runs) {
        const saved = await store.insertIfAbsent(run)
        ;(saved ? imported : skipped).push(run.id)
      }
      return c.json({ imported, skipped })
    })
    .get('/:runId', zValidator('param', IdParamSchema), async (c) => {
      const run = await store.get(c.req.valid('param').runId)
      if (!run) return c.json({ error: 'Unknown run' }, 404)
      return c.json({ run })
    })
    .put(
      '/:runId',
      zValidator('param', IdParamSchema),
      zValidator('json', UpsertRunSchema),
      async (c) => {
        const run = await store.upsert({
          ...c.req.valid('json'),
          id: c.req.valid('param').runId,
        })
        // Every write, not just the first: a run is written twice, when it
        // starts and when it finishes, and pruning is bounded and idempotent.
        // The import path deliberately does not prune, so it stays additive.
        await store.prune(run.jobId)
        return c.json({ run })
      },
    )
    .delete('/:runId', zValidator('param', IdParamSchema), async (c) => {
      const deleted = await store.remove(c.req.valid('param').runId)
      if (!deleted) return c.json({ error: 'Unknown run' }, 404)
      return c.json({ success: true })
    })
}
