/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { and, eq, ne, sql } from 'drizzle-orm'
import type { AnySQLiteColumn } from 'drizzle-orm/sqlite-core'
import { getDb } from '../db'
import { type NewProviderRow, type ProviderRow, providers } from '../db/schema'

/**
 * Every column except the four that hold secrets, plus flags saying whether
 * each is set so the UI can still show that a key exists.
 *
 * Drizzle has no view, but naming the columns gives the same guarantee: a
 * caller of the public reads cannot receive a credential even by accident,
 * where a `select()` would hand them out on every list, get and default.
 */
function isSet(column: AnySQLiteColumn) {
  return sql<boolean>`${column} IS NOT NULL AND ${column} <> ''`.mapWith(
    Boolean,
  )
}

const publicColumns = {
  id: providers.id,
  profileId: providers.profileId,
  kind: providers.kind,
  type: providers.type,
  name: providers.name,
  modelId: providers.modelId,
  reasoningEffort: providers.reasoningEffort,
  isDefault: providers.isDefault,
  createdAt: providers.createdAt,
  updatedAt: providers.updatedAt,
  baseUrl: providers.baseUrl,
  headers: providers.headers,
  supportsImages: providers.supportsImages,
  contextWindow: providers.contextWindow,
  temperature: providers.temperature,
  resourceName: providers.resourceName,
  region: providers.region,
  reasoningSummary: providers.reasoningSummary,
  workingDirectory: providers.workingDirectory,
  customConfig: providers.customConfig,
  // Empty counts as unset, matching what the upsert treats as not supplied.
  // Otherwise a blank field would read back as a stored credential.
  hasApiKey: isSet(providers.apiKey),
  hasAccessKeyId: isSet(providers.accessKeyId),
  hasSecretAccessKey: isSet(providers.secretAccessKey),
  hasSessionToken: isSet(providers.sessionToken),
}

export type PublicProviderRow = {
  [K in keyof typeof publicColumns]: K extends `has${string}`
    ? boolean
    : ProviderRow[Extract<K, keyof ProviderRow>]
}

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
  /** Every provider, whatever its kind, without credentials. */
  list(): Promise<PublicProviderRow[]>
  /** Only the LLM providers, without credentials. */
  listLlm(): Promise<PublicProviderRow[]>
  get(id: string): Promise<PublicProviderRow | null>
  /**
   * The full row, credentials included. Only for callers inside the server
   * that have to build an outbound request, never for a route response.
   */
  getWithCredentials(id: string): Promise<ProviderRow | null>
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
  getDefault(): Promise<PublicProviderRow | null>
  /** The selected provider with its credentials, for the same callers as above. */
  getDefaultWithCredentials(): Promise<ProviderRow | null>
  /**
   * Points the default at one provider of any kind. Returns false when the id
   * is unknown, so a stale pointer cannot be stored.
   */
  setDefault(id: string): Promise<boolean>
}

async function list(): Promise<PublicProviderRow[]> {
  return getDb().select(publicColumns).from(providers).all()
}

async function listLlm(): Promise<PublicProviderRow[]> {
  return getDb()
    .select(publicColumns)
    .from(providers)
    .where(eq(providers.kind, 'llm'))
    .all()
}

async function get(id: string): Promise<PublicProviderRow | null> {
  const [row] = await getDb()
    .select(publicColumns)
    .from(providers)
    .where(eq(providers.id, id))
    .limit(1)
  return row ?? null
}

async function getWithCredentials(id: string): Promise<ProviderRow | null> {
  const [row] = await getDb()
    .select()
    .from(providers)
    .where(eq(providers.id, id))
    .limit(1)
  return row ?? null
}

const CREDENTIAL_FIELDS = [
  'apiKey',
  'accessKeyId',
  'secretAccessKey',
  'sessionToken',
] as const

/**
 * Drops credential fields the caller did not supply, so they keep their stored
 * value.
 *
 * Reads no longer return credentials, so a client editing a provider cannot
 * send back what it never received. A plain whole-row upsert would then write
 * over a working key on every rename.
 *
 * An empty string counts as not supplied, not as an instruction to clear. A
 * form field that was never filled in submits as `''` rather than undefined,
 * so treating the two differently would wipe the key on exactly the edit this
 * exists to protect. Clearing is deliberate and explicit: send null.
 */
function withoutAbsentCredentials<T extends Record<string, unknown>>(
  row: T,
): Partial<T> {
  const next: Record<string, unknown> = { ...row }
  for (const field of CREDENTIAL_FIELDS) {
    if (next[field] === undefined || next[field] === '') delete next[field]
  }
  return next as Partial<T>
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
        ...withoutAbsentCredentials(values),
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

async function getDefault(): Promise<PublicProviderRow | null> {
  const [row] = await getDb()
    .select(publicColumns)
    .from(providers)
    .where(eq(providers.isDefault, true))
    .limit(1)
  return row ?? null
}

async function getDefaultWithCredentials(): Promise<ProviderRow | null> {
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
  getWithCredentials,
  upsert,
  insertIfAbsent,
  remove,
  getDefault,
  getDefaultWithCredentials,
  setDefault,
}
