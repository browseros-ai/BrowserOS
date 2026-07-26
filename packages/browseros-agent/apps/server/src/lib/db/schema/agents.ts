/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { InferInsertModel, InferSelectModel } from 'drizzle-orm'
import {
  index,
  integer,
  sqliteTable,
  text,
  uniqueIndex,
} from 'drizzle-orm/sqlite-core'

export const agentDefinitions = sqliteTable(
  'agent_definitions',
  {
    id: text('id').primaryKey(),
    name: text('name').notNull(),
    adapter: text('adapter', {
      enum: ['claude', 'codex'],
    }).notNull(),
    modelId: text('model_id').notNull(),
    reasoningEffort: text('reasoning_effort').notNull(),
    permissionMode: text('permission_mode', {
      enum: ['approve-all'],
    })
      .notNull()
      .default('approve-all'),
    sessionKey: text('session_key').notNull(),
    pinned: integer('pinned', { mode: 'boolean' }).notNull().default(false),
    adapterConfigJson: text('adapter_config_json'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => [
    uniqueIndex('agent_definitions_session_key_unique').on(table.sessionKey),
    index('agent_definitions_updated_at_idx').on(table.updatedAt),
    index('agent_definitions_adapter_updated_at_idx').on(
      table.adapter,
      table.updatedAt,
    ),
  ],
)

export type AgentDefinitionRow = InferSelectModel<typeof agentDefinitions>
export type NewAgentDefinitionRow = InferInsertModel<typeof agentDefinitions>
