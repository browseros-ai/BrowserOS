/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { InferInsertModel, InferSelectModel } from 'drizzle-orm'
import { index, integer, primaryKey, sqliteTable, text } from 'drizzle-orm/sqlite-core'

export const assistantSessions = sqliteTable(
  'assistant_sessions',
  {
    id: text('id').primaryKey(),
    title: text('title'),
    mode: text('mode', { enum: ['chat', 'agent'] })
      .notNull()
      .default('chat'),
    model: text('model'),
    messageCount: integer('message_count').notNull().default(0),
    lastMessagePreview: text('last_message_preview'),
    lastMessageAt: integer('last_message_at'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
    meta: text('meta'),
  },
  (table) => [
    index('assistant_sessions_updated_at_idx').on(table.updatedAt),
  ],
)

export const sessionWorkspaces = sqliteTable(
  'session_workspaces',
  {
    sessionId: text('session_id').notNull(),
    workspaceId: text('workspace_id').notNull(),
    workspacePath: text('workspace_path').notNull(),
    workspaceName: text('workspace_name').notNull(),
  },
  (table) => [
    primaryKey({ columns: [table.sessionId, table.workspaceId] }),
    index('session_workspaces_session_id_idx').on(table.sessionId),
    index('session_workspaces_workspace_path_idx').on(table.workspacePath),
  ],
)

export const sessionTags = sqliteTable(
  'session_tags',
  {
    sessionId: text('session_id').notNull(),
    tag: text('tag').notNull(),
  },
  (table) => [
    index('session_tags_tag_idx').on(table.tag),
  ],
)

export type AssistantSessionRow = InferSelectModel<typeof assistantSessions>
export type NewAssistantSessionRow = InferInsertModel<typeof assistantSessions>

export type SessionWorkspaceRow = InferSelectModel<typeof sessionWorkspaces>
export type NewSessionWorkspaceRow = InferInsertModel<typeof sessionWorkspaces>

export type SessionTagRow = InferSelectModel<typeof sessionTags>
export type NewSessionTagRow = InferInsertModel<typeof sessionTags>
