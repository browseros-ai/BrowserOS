/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { InferInsertModel, InferSelectModel } from 'drizzle-orm'
import { sql } from 'drizzle-orm'
import {
  check,
  index,
  integer,
  real,
  sqliteTable,
  text,
  uniqueIndex,
} from 'drizzle-orm/sqlite-core'

/**
 * Everything the chat can be pointed at, in one table.
 *
 * LLM providers and ACP agents were separate tables, but both are simply
 * providers for a conversation and everything above the database already said
 * so: the chat target is one union with a kind on the client, and the wire
 * target has always been a discriminated union. Keeping them apart meant two
 * selection pointers that could disagree, and a scheduled job that could only
 * ever reference an LLM provider.
 *
 * `kind` carries the distinction. Columns that only one kind uses are nullable
 * and grouped below, with the per-kind requirements held by a check constraint
 * rather than by the column definitions.
 *
 * `profileId` is reserved and currently always null, as in the other tables.
 */
export const providers = sqliteTable(
  'providers',
  {
    id: text('id').primaryKey(),
    profileId: text('profile_id'),
    kind: text('kind', { enum: ['llm', 'acp'] }).notNull(),
    type: text('type').notNull(),
    name: text('name').notNull(),
    /** Required for an llm provider, optional for an acp agent. */
    modelId: text('model_id'),
    reasoningEffort: text('reasoning_effort'),
    /** At most one row may set this; the unique index below enforces it. */
    isDefault: integer('is_default', { mode: 'boolean' })
      .notNull()
      .default(false),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),

    baseUrl: text('base_url'),
    supportsImages: integer('supports_images', { mode: 'boolean' })
      .notNull()
      .default(true),
    contextWindow: integer('context_window'),
    // Real, not integer: the default is 0.2 and an integer column floors it to 0.
    temperature: real('temperature').notNull().default(0.2),
    apiKey: text('api_key'),
    accessKeyId: text('access_key_id'),
    secretAccessKey: text('secret_access_key'),
    sessionToken: text('session_token'),
    resourceName: text('resource_name'),
    region: text('region'),
    reasoningSummary: text('reasoning_summary'),

    workingDirectory: text('working_directory'),
    customConfig: text('custom_config'),
  },
  (table) => [
    index('providers_profile_id_idx').on(table.profileId),
    index('providers_kind_updated_at_idx').on(table.kind, table.updatedAt),
    // Every row in this index has is_default = 1, so uniqueness on that single
    // column admits exactly one default row.
    //
    // Deliberately not keyed by profile_id. SQLite treats NULLs as distinct in
    // a unique index, so a (profile_id, is_default) pair would let every row be
    // default at once while profile_id is unset, which is its state today. The
    // per-profile form belongs in the migration that starts populating
    // profile_id, not in this one.
    uniqueIndex('providers_one_default')
      .on(table.isDefault)
      .where(sql`${table.isDefault} = 1`),
    check(
      'providers_llm_requires_model_and_context',
      sql`${table.kind} <> 'llm' OR (${table.modelId} IS NOT NULL AND ${table.contextWindow} IS NOT NULL)`,
    ),
  ],
)

export type ProviderRow = InferSelectModel<typeof providers>
export type NewProviderRow = InferInsertModel<typeof providers>
