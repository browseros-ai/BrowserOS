/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { eq } from 'drizzle-orm'
import { getDb } from '../db'
import {
  type LlmProviderRow,
  llmProviders,
  type NewLlmProviderRow,
} from '../db/schema'

/**
 * The store stamps `updatedAt` and defaults `createdAt`, so callers supply
 * neither. `createdAt` stays optional so an import can preserve the original
 * creation time when it has one.
 */
export type LlmProviderUpsert = Omit<
  NewLlmProviderRow,
  'updatedAt' | 'createdAt'
> & {
  createdAt?: number
}

export interface LlmProviderStore {
  list(): Promise<LlmProviderRow[]>
  get(id: string): Promise<LlmProviderRow | null>
  /** Insert or replace by id. This is the app's ordinary write path. */
  upsert(row: LlmProviderUpsert): Promise<LlmProviderRow>
  /**
   * Insert only when the id is absent; returns null when a row already exists.
   *
   * The one-time import uses this rather than `upsert` because the app writes
   * directly to this table as well. A second import run must never replace a
   * provider the user has edited since with the stale copy still sitting in
   * extension storage.
   */
  insertIfAbsent(row: LlmProviderUpsert): Promise<LlmProviderRow | null>
  remove(id: string): Promise<boolean>
}

async function list(): Promise<LlmProviderRow[]> {
  return getDb().select().from(llmProviders).all()
}

async function get(id: string): Promise<LlmProviderRow | null> {
  const [row] = await getDb()
    .select()
    .from(llmProviders)
    .where(eq(llmProviders.id, id))
    .limit(1)
  return row ?? null
}

async function upsert(row: LlmProviderUpsert): Promise<LlmProviderRow> {
  const now = Date.now()
  const [saved] = await getDb()
    .insert(llmProviders)
    .values({ ...row, createdAt: row.createdAt ?? now, updatedAt: now })
    .onConflictDoUpdate({
      target: llmProviders.id,
      // createdAt is deliberately absent: re-importing a provider must not
      // rewrite when the user originally created it.
      set: { ...row, createdAt: undefined, updatedAt: now },
    })
    .returning()
  return saved
}

async function insertIfAbsent(
  row: LlmProviderUpsert,
): Promise<LlmProviderRow | null> {
  const now = Date.now()
  // onConflictDoNothing returns no row on conflict, so the absent/present
  // decision and the write are one statement rather than a select then insert.
  const [saved] = await getDb()
    .insert(llmProviders)
    .values({ ...row, createdAt: row.createdAt ?? now, updatedAt: now })
    .onConflictDoNothing({ target: llmProviders.id })
    .returning()
  return saved ?? null
}

async function remove(id: string): Promise<boolean> {
  const deleted = await getDb()
    .delete(llmProviders)
    .where(eq(llmProviders.id, id))
    .returning({ id: llmProviders.id })
  return deleted.length > 0
}

export const dbLlmProviderStore: LlmProviderStore = {
  list,
  get,
  upsert,
  insertIfAbsent,
  remove,
}
