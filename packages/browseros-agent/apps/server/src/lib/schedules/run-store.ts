/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { desc, eq, inArray } from 'drizzle-orm'
import { getDb } from '../db'
import {
  type NewScheduledJobRunRow,
  type ScheduledJobRunRow,
  scheduledJobRuns,
} from '../db/schema'

/**
 * The store stamps `updatedAt` and defaults `createdAt`, so callers supply
 * neither. `createdAt` stays optional so an import can preserve the original
 * creation time when it has one.
 */
export type ScheduledJobRunUpsert = Omit<
  NewScheduledJobRunRow,
  'updatedAt' | 'createdAt'
> & {
  createdAt?: number
}

/**
 * Runs kept per job. The extension applied this cap when it owned the history,
 * trimming as it created each run; keeping the number here means it holds
 * however the run was written rather than only on the path that happened to
 * enforce it.
 */
export const MAX_RUNS_PER_JOB = 15

export interface ScheduledJobRunStore {
  list(): Promise<ScheduledJobRunRow[]>
  get(id: string): Promise<ScheduledJobRunRow | null>
  /** Insert or replace by id. A run is written once when it starts and again
   * when it finishes, so this is the ordinary write path. */
  upsert(row: ScheduledJobRunUpsert): Promise<ScheduledJobRunRow>
  /** Insert only when the id is absent; returns null when a row already
   * exists. Used by the one-time import for the reason on the provider store. */
  insertIfAbsent(row: ScheduledJobRunUpsert): Promise<ScheduledJobRunRow | null>
  remove(id: string): Promise<boolean>
  /** Drops all but the newest `keep` runs of a job. Returns how many went. */
  prune(jobId: string, keep?: number): Promise<number>
}

async function list(): Promise<ScheduledJobRunRow[]> {
  return getDb()
    .select()
    .from(scheduledJobRuns)
    .orderBy(desc(scheduledJobRuns.startedAt))
    .all()
}

async function get(id: string): Promise<ScheduledJobRunRow | null> {
  const [row] = await getDb()
    .select()
    .from(scheduledJobRuns)
    .where(eq(scheduledJobRuns.id, id))
    .limit(1)
  return row ?? null
}

async function upsert(row: ScheduledJobRunUpsert): Promise<ScheduledJobRunRow> {
  const now = Date.now()
  const [saved] = await getDb()
    .insert(scheduledJobRuns)
    .values({ ...row, createdAt: row.createdAt ?? now, updatedAt: now })
    .onConflictDoUpdate({
      target: scheduledJobRuns.id,
      set: { ...row, createdAt: undefined, updatedAt: now },
    })
    .returning()
  return saved
}

async function insertIfAbsent(
  row: ScheduledJobRunUpsert,
): Promise<ScheduledJobRunRow | null> {
  const now = Date.now()
  const [saved] = await getDb()
    .insert(scheduledJobRuns)
    .values({ ...row, createdAt: row.createdAt ?? now, updatedAt: now })
    .onConflictDoNothing({ target: scheduledJobRuns.id })
    .returning()
  return saved ?? null
}

async function prune(
  jobId: string,
  keep: number = MAX_RUNS_PER_JOB,
): Promise<number> {
  const rows = await getDb()
    .select({ id: scheduledJobRuns.id })
    .from(scheduledJobRuns)
    .where(eq(scheduledJobRuns.jobId, jobId))
    .orderBy(desc(scheduledJobRuns.startedAt))
    .all()

  const stale = rows.slice(keep).map((row) => row.id)
  if (stale.length === 0) return 0

  await getDb()
    .delete(scheduledJobRuns)
    .where(inArray(scheduledJobRuns.id, stale))
  return stale.length
}

async function remove(id: string): Promise<boolean> {
  const deleted = await getDb()
    .delete(scheduledJobRuns)
    .where(eq(scheduledJobRuns.id, id))
    .returning({ id: scheduledJobRuns.id })
  return deleted.length > 0
}

export const dbScheduledJobRunStore: ScheduledJobRunStore = {
  list,
  get,
  upsert,
  insertIfAbsent,
  remove,
  prune,
}
