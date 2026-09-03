/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { InferInsertModel, InferSelectModel } from 'drizzle-orm'
import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'
import { scheduledJobs } from './scheduled-jobs'

/**
 * One tool invocation recorded during a run, as the extension shapes it.
 *
 * `input` is optional here where the extension has it required. An `unknown`
 * already admits undefined, so the two describe the same values, and zod infers
 * a key of that type as optional. Matching the validator keeps this honest
 * rather than asserting the difference away at the route boundary.
 */
export interface ToolCallExecution {
  id: string
  name: string
  input?: unknown
  output?: unknown
  error?: string
  timestamp: string
}

/**
 * History of scheduled job executions.
 *
 * Cascades on job delete, unlike the job to provider reference which is
 * `set null`. A job whose provider was removed is a job needing attention; a
 * run whose job was removed means nothing, and deleting a job already removed
 * its runs before this table existed.
 *
 * Timestamps are epoch integers here while the extension holds ISO strings,
 * matching the other tables. The route layer converts.
 */
export const scheduledJobRuns = sqliteTable(
  'scheduled_job_runs',
  {
    id: text('id').primaryKey(),
    profileId: text('profile_id'),
    jobId: text('job_id')
      .notNull()
      .references(() => scheduledJobs.id, { onDelete: 'cascade' }),
    status: text('status', {
      enum: ['running', 'completed', 'failed'],
    }).notNull(),
    startedAt: integer('started_at').notNull(),
    completedAt: integer('completed_at'),
    result: text('result'),
    finalResult: text('final_result'),
    executionLog: text('execution_log'),
    toolCalls: text('tool_calls', { mode: 'json' }).$type<
      ToolCallExecution[]
    >(),
    error: text('error'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => [
    index('scheduled_job_runs_job_id_idx').on(table.jobId),
    index('scheduled_job_runs_started_at_idx').on(table.startedAt),
  ],
)

export type ScheduledJobRunRow = InferSelectModel<typeof scheduledJobRuns>
export type NewScheduledJobRunRow = InferInsertModel<typeof scheduledJobRuns>
