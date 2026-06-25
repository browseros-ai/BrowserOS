/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * One row per MCP session shutdown: written from the transport's
 * `onsessionclosed` (kind='closed') or `onerror` (kind='errored').
 * Drives task status semantics: a session with an ends row + no
 * error rows is Done; a session with kind='errored' or any
 * dispatch carrying isError=true is Failed; a session with no
 * ends row is Live until it goes idle past the deriver's threshold.
 */

import { sql } from 'drizzle-orm'
import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

export const agentSessionEnds = sqliteTable(
  'agent_session_ends',
  {
    id: integer('id').primaryKey({ autoIncrement: true }),
    createdAt: integer('created_at')
      .notNull()
      .default(sql`(unixepoch('subsec') * 1000)`),
    sessionId: text('session_id').notNull(),
    kind: text('kind', { enum: ['closed', 'errored'] }).notNull(),
    reason: text('reason'),
  },
  (t) => ({
    sessionIdx: index('agent_session_ends_session_idx').on(t.sessionId),
    createdAtIdx: index('agent_session_ends_created_at_idx').on(t.createdAt),
  }),
)

export type AgentSessionEndRow = typeof agentSessionEnds.$inferSelect
export type NewAgentSessionEnd = typeof agentSessionEnds.$inferInsert
