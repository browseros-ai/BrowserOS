/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { UIMessage } from 'ai'
import type { InferInsertModel, InferSelectModel } from 'drizzle-orm'
import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

export const conversations = sqliteTable(
  'conversations',
  {
    id: text('id').primaryKey(),
    messages: text('messages', { mode: 'json' }).$type<UIMessage[]>().notNull(),
    lastUserMessage: text('last_user_message'),
    origin: text('origin'),
    targetType: text('target_type', {
      enum: ['browseros', 'claude', 'codex'],
    }).notNull(),
    agentId: text('agent_id'),
    lastMessagedAt: integer('last_messaged_at').notNull(),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => [
    index('conversations_last_messaged_at_idx').on(table.lastMessagedAt),
  ],
)

export type ConversationRow = InferSelectModel<typeof conversations>
export type NewConversationRow = InferInsertModel<typeof conversations>
