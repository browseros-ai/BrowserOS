/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { eq } from 'drizzle-orm'
import { getDb } from '../db'
import {
  type NewScheduledJobRow,
  type ScheduledJobRow,
  scheduledJobs,
} from '../db/schema'

/**
 * The store stamps `updatedAt` and defaults `createdAt`, so callers supply
 * neither. `createdAt` stays optional so an import can preserve the original
 * creation time when it has one.
 */
export type ScheduledJobUpsert = Omit<
  NewScheduledJobRow,
  'updatedAt' | 'createdAt'
> & {
  createdAt?: number
}

export interface ScheduledJobStore {
  list(): Promise<ScheduledJobRow[]>
  get(id: string): Promise<ScheduledJobRow | null>
  /** Insert or replace by id. The migration relies on this being idempotent. */
  upsert(row: ScheduledJobUpsert): Promise<ScheduledJobRow>
  remove(id: string): Promise<boolean>
}

async function list(): Promise<ScheduledJobRow[]> {
  return getDb().select().from(scheduledJobs).all()
}

async function get(id: string): Promise<ScheduledJobRow | null> {
  const [row] = await getDb()
    .select()
    .from(scheduledJobs)
    .where(eq(scheduledJobs.id, id))
    .limit(1)
  return row ?? null
}

async function upsert(row: ScheduledJobUpsert): Promise<ScheduledJobRow> {
  const now = Date.now()
  const [saved] = await getDb()
    .insert(scheduledJobs)
    .values({ ...row, createdAt: row.createdAt ?? now, updatedAt: now })
    .onConflictDoUpdate({
      target: scheduledJobs.id,
      set: { ...row, createdAt: undefined, updatedAt: now },
    })
    .returning()
  return saved
}

async function remove(id: string): Promise<boolean> {
  const deleted = await getDb()
    .delete(scheduledJobs)
    .where(eq(scheduledJobs.id, id))
    .returning({ id: scheduledJobs.id })
  return deleted.length > 0
}

export const dbScheduledJobStore: ScheduledJobStore = {
  list,
  get,
  upsert,
  remove,
}
