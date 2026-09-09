/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { and, asc, desc, eq, inArray, ne, sql } from 'drizzle-orm'
import type { AnySQLiteColumn } from 'drizzle-orm/sqlite-core'
import { getDb } from '../db'
import {
  type NewProviderRow,
  type ProviderRow,
  providers,
  scheduledJobs,
} from '../db/schema'
import {
  ProviderConfigError,
  providerBaseUrl,
  resolveProviderConfig,
  SINGLE_INSTANCE_PROVIDERS,
} from './provider-config'

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

/** Write responses carry the same redaction contract as list/get responses. */
export function publicProvider(row: ProviderRow): PublicProviderRow {
  const {
    apiKey,
    accessKeyId,
    secretAccessKey,
    sessionToken,
    ...publicFields
  } = row
  return {
    ...publicFields,
    hasApiKey: !!apiKey,
    hasAccessKeyId: !!accessKeyId,
    hasSecretAccessKey: !!secretAccessKey,
    hasSessionToken: !!sessionToken,
  }
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
  /** Resolve, validate and save atomically; OAuth reconnects keep their existing id. */
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

async function upsert(row: ProviderUpsert): Promise<ProviderRow> {
  // Reads, credential reuse and singleton reconciliation share a transaction:
  // two UI surfaces saving concurrently must not use stale configuration or
  // choose different ids for the same OAuth account.
  return getDb().transaction(
    (tx) => {
      const byId = tx
        .select()
        .from(providers)
        .where(eq(providers.id, row.id))
        .get()
      if (byId && byId.kind !== 'llm')
        throw new ProviderConfigError({
          type: 'This ID belongs to a coding agent.',
        })
      const siblings = SINGLE_INSTANCE_PROVIDERS.has(row.type)
        ? tx
            .select()
            .from(providers)
            .where(and(eq(providers.kind, 'llm'), eq(providers.type, row.type)))
            .orderBy(
              desc(providers.isDefault),
              asc(providers.createdAt),
              asc(providers.id),
            )
            .all()
        : []
      const existing = byId ?? siblings[0] ?? null
      const id = existing?.id ?? row.id
      const config = resolveProviderConfig({ ...row, id }, existing)
      const removedIds = siblings
        .filter((sibling) => sibling.id !== id)
        .map((sibling) => sibling.id)
      const isDefault =
        !!existing?.isDefault || siblings.some((sibling) => sibling.isDefault)
      if (removedIds.length) {
        // Keep scheduled jobs attached when collapsing duplicates from older
        // clients. Move their references before deleting any provider row.
        tx.update(scheduledJobs)
          .set({ providerId: id })
          .where(inArray(scheduledJobs.providerId, removedIds))
          .run()
        tx.delete(providers).where(inArray(providers.id, removedIds)).run()
      }
      const now = Date.now()
      const values = {
        ...config,
        kind: 'llm' as const,
        isDefault,
        createdAt: existing?.createdAt ?? row.createdAt ?? now,
        updatedAt: now,
      }
      return tx
        .insert(providers)
        .values(values)
        .onConflictDoUpdate({ target: providers.id, set: values })
        .returning()
        .get()
    },
    { behavior: 'immediate' },
  )
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
      baseUrl: providerBaseUrl(row) || null,
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
