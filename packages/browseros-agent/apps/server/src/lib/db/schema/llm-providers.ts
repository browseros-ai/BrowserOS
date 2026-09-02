/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { InferInsertModel, InferSelectModel } from 'drizzle-orm'
import {
  index,
  integer,
  real,
  sqliteTable,
  text,
} from 'drizzle-orm/sqlite-core'

/**
 * LLM providers, mirroring the shape the extension holds today.
 *
 * Credentials live here in the clear, alongside the OAuth tokens already in
 * this database. Nothing beyond filesystem permissions protects them, which is
 * the same posture they had in extension storage.
 *
 * `profileId` is reserved and currently always null: every browser profile on
 * a machine shares one database, because no extension API exposes a profile
 * identifier. The column exists so isolation can be switched on later without
 * a second migration.
 */
export const llmProviders = sqliteTable(
  'llm_providers',
  {
    id: text('id').primaryKey(),
    profileId: text('profile_id'),
    type: text('type').notNull(),
    name: text('name').notNull(),
    baseUrl: text('base_url'),
    modelId: text('model_id').notNull(),
    supportsImages: integer('supports_images', { mode: 'boolean' })
      .notNull()
      .default(true),
    contextWindow: integer('context_window').notNull(),
    // Real, not integer: the default is 0.2 and an integer column floors it to 0.
    temperature: real('temperature').notNull().default(0.2),

    apiKey: text('api_key'),
    accessKeyId: text('access_key_id'),
    secretAccessKey: text('secret_access_key'),
    sessionToken: text('session_token'),

    resourceName: text('resource_name'),
    region: text('region'),

    reasoningEffort: text('reasoning_effort'),
    reasoningSummary: text('reasoning_summary'),

    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => [index('llm_providers_profile_id_idx').on(table.profileId)],
)

export type LlmProviderRow = InferSelectModel<typeof llmProviders>
export type NewLlmProviderRow = InferInsertModel<typeof llmProviders>
