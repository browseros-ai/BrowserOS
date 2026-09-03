/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { and, eq, ne } from 'drizzle-orm'
import { getDb } from '../db'
import { type NewProviderRow, type ProviderRow, providers } from '../db/schema'

/**
 * The store stamps `updatedAt` and defaults `createdAt`, so callers supply
 * neither. `createdAt` stays optional so an import can preserve the original
 * creation time when it has one.
 */
export type ProviderUpsert = Omit<
  NewProviderRow,
  'updatedAt' | 'createdAt' | 'kind'
> & {
  createdAt?: number
}

export interface ProviderStore {
  /** Every provider, whatever its kind. */
  list(): Promise<ProviderRow[]>
  /** Only the LLM providers, for the surfaces that still separate them. */
  listLlm(): Promise<ProviderRow[]>
  get(id: string): Promise<ProviderRow | null>
  /** Insert or replace by id. This is the app's ordinary write path. */
  upsert(row: ProviderUpsert): Promise<ProviderRow>
  /**
   * Insert only when the id is absent; returns null when a row already exists.
   *
   * The one-time import uses this rather than `upsert` because the app writes
   * to this table directly as well. A second import run must never replace a
   * provider the user has edited since with the stale copy still sitting in
   * extension storage.
   */
  insertIfAbsent(row: ProviderUpsert): Promise<ProviderRow | null>
  remove(id: string): Promise<boolean>
  /** The one selected provider, of any kind, or null when none is set. */
  getDefault(): Promise<ProviderRow | null>
  /**
   * Points the default at one provider of any kind. Returns false when the id
   * is unknown, so a stale pointer cannot be stored.
   */
  setDefault(id: string): Promise<boolean>
}

async function list(): Promise<ProviderRow[]> {
  return getDb().select().from(providers).all()
}

async function listLlm(): Promise<ProviderRow[]> {
  return getDb().select().from(providers).where(eq(providers.kind, 'llm')).all()
}

async function get(id: string): Promise<ProviderRow | null> {
  const [row] = await getDb()
    .select()
    .from(providers)
    .where(eq(providers.id, id))
    .limit(1)
  return row ?? null
}

async function upsert(row: ProviderUpsert): Promise<ProviderRow> {
  const now = Date.now()
  const values = { ...row, kind: 'llm' as const }
  const [saved] = await getDb()
    .insert(providers)
    .values({ ...values, createdAt: row.createdAt ?? now, updatedAt: now })
    .onConflictDoUpdate({
      target: providers.id,
      // createdAt is deliberately absent: re-importing a provider must not
      // rewrite when the user originally created it. isDefault likewise, so a
      // save does not silently move the selection.
      set: {
        ...values,
        createdAt: undefined,
        isDefault: undefined,
        updatedAt: now,
      },
    })
    .returning()
  return saved
}

async function insertIfAbsent(
  row: ProviderUpsert,
): Promise<ProviderRow | null> {
  const now = Date.now()
  // onConflictDoNothing returns no row on conflict, so the absent/present
  // decision and the write are one statement rather than a select then insert.
  const [saved] = await getDb()
    .insert(providers)
    .values({
      ...row,
      kind: 'llm' as const,
      createdAt: row.createdAt ?? now,
      updatedAt: now,
    })
    .onConflictDoNothing({ target: providers.id })
    .returning()
  return saved ?? null
}

async function remove(id: string): Promise<boolean> {
  const deleted = await getDb()
    .delete(providers)
    .where(eq(providers.id, id))
    .returning({ id: providers.id })
  return deleted.length > 0
}

async function getDefault(): Promise<ProviderRow | null> {
  const [row] = await getDb()
    .select()
    .from(providers)
    .where(eq(providers.isDefault, true))
    .limit(1)
  return row ?? null
}

async function setDefault(id: string): Promise<boolean> {
  const target = await get(id)
  if (!target) return false

  // Clearing first is required, not tidiness: a partial unique index allows one
  // row with is_default = 1, so setting the new one before clearing the old
  // would violate it.
  return getDb().transaction((tx) => {
    tx.update(providers)
      .set({ isDefault: false })
      .where(and(eq(providers.isDefault, true), ne(providers.id, id)))
      .run()
    tx.update(providers)
      .set({ isDefault: true })
      .where(eq(providers.id, id))
      .run()
    return true
  })
}

export const dbProviderStore: ProviderStore = {
  list,
  listLlm,
  get,
  upsert,
  insertIfAbsent,
  remove,
  getDefault,
  setDefault,
}
