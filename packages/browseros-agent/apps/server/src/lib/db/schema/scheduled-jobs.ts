/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { InferInsertModel, InferSelectModel } from 'drizzle-orm'
import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'
import { providers } from './providers'

/**
 * Scheduled jobs, mirroring the shape the extension holds today.
 *
 * `providerId` points at the unified providers table, so a job can target an
 * ACP agent as readily as an LLM provider. While the two were separate tables
 * this reference could only ever name an LLM provider.
 *
 * The reference is deliberately not a foreign key with a cascade: a job whose
 * provider was deleted should surface as a job needing attention, not vanish
 * silently on a delete the user made elsewhere.
 *
 * Timestamps are epoch integers here while the extension holds ISO strings.
 * The database is internally consistent this way, and the route layer converts.
 */
export const scheduledJobs = sqliteTable(
  'scheduled_jobs',
  {
    id: text('id').primaryKey(),
    profileId: text('profile_id'),
    name: text('name').notNull(),
    query: text('query').notNull(),
    scheduleType: text('schedule_type', {
      enum: ['daily', 'hourly', 'minutes'],
    }).notNull(),
    scheduleTime: text('schedule_time'),
    scheduleInterval: integer('schedule_interval'),
    enabled: integer('enabled', { mode: 'boolean' }).notNull().default(true),
    providerId: text('provider_id').references(() => providers.id, {
      onDelete: 'set null',
    }),
    lastRunAt: integer('last_run_at'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => [
    index('scheduled_jobs_profile_id_idx').on(table.profileId),
    index('scheduled_jobs_enabled_idx').on(table.enabled),
  ],
)

export type ScheduledJobRow = InferSelectModel<typeof scheduledJobs>
export type NewScheduledJobRow = InferInsertModel<typeof scheduledJobs>
