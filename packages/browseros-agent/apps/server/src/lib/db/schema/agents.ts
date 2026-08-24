/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { InferInsertModel, InferSelectModel } from 'drizzle-orm'
import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

export const acpAgents = sqliteTable(
  'acp_agents',
  {
    id: text('id').primaryKey(),
    name: text('name').notNull(),
    type: text('type', { enum: ['claude', 'codex', 'custom'] }).notNull(),
    modelId: text('model_id'),
    reasoningEffort: text('reasoning_effort'),
    workingDirectory: text('working_directory'),
    customConfig: text('custom_config'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => [
    index('acp_agents_updated_at_idx').on(table.updatedAt),
    index('acp_agents_type_updated_at_idx').on(table.type, table.updatedAt),
  ],
)

export type AcpAgentRow = InferSelectModel<typeof acpAgents>
export type NewAcpAgentRow = InferInsertModel<typeof acpAgents>
