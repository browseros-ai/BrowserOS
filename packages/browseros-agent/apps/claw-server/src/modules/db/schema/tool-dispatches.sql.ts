/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 audit log: one row per successful tool dispatch. Snapshot fields
 * (agentLabel, url, title) are captured at dispatch time so renames
 * and navigations later do not rewrite the history operators read.
 * The dispatch hook in `register.ts` writes here best-effort; failures
 * never block the agent.
 */

import { sql } from 'drizzle-orm'
import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

export const toolDispatches = sqliteTable(
  'tool_dispatches',
  {
    id: integer('id').primaryKey({ autoIncrement: true }),
    createdAt: integer('created_at')
      .notNull()
      .default(sql`(unixepoch('subsec') * 1000)`),
    agentId: text('agent_id').notNull(),
    slug: text('slug').notNull(),
    agentLabel: text('agent_label').notNull(),
    sessionId: text('session_id').notNull(),
    toolName: text('tool_name').notNull(),
    pageId: integer('page_id'),
    targetId: text('target_id'),
    url: text('url'),
    title: text('title'),
    argsJson: text('args_json'),
    resultMeta: text('result_meta'),
    durationMs: integer('duration_ms'),
  },
  (t) => ({
    createdAtIdx: index('tool_dispatches_created_at_idx').on(t.createdAt),
    agentCreatedIdx: index('tool_dispatches_agent_created_idx').on(
      t.agentId,
      t.createdAt,
    ),
    sessionIdx: index('tool_dispatches_session_idx').on(t.sessionId),
  }),
)

export type ToolDispatchRow = typeof toolDispatches.$inferSelect
export type ToolDispatchInsert = typeof toolDispatches.$inferInsert
