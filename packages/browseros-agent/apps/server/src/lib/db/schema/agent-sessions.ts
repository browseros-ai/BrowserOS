/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { InferInsertModel, InferSelectModel } from 'drizzle-orm'
import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

export const agentSessions = sqliteTable(
  'agent_sessions',
  {
    id: text('id').primaryKey(),
    agentId: text('agent_id').notNull(),
    title: text('title'),
    mode: text('mode', { enum: ['code', 'ask', 'agent'] })
      .notNull()
      .default('agent'),
    model: text('model'),
    turnCount: integer('turn_count').notNull().default(0),
    lastMessagePreview: text('last_message_preview'),
    lastMessageAt: integer('last_message_at'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
    meta: text('meta'),
  },
  (table) => [
    index('agent_sessions_agent_updated_idx').on(table.agentId, table.updatedAt),
  ],
)

export type AgentSessionRow = InferSelectModel<typeof agentSessions>
export type NewAgentSessionRow = InferInsertModel<typeof agentSessions>
