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

import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

export const agentSessionEnds = sqliteTable(
  'agent_session_ends',
  {
    id: integer('id').primaryKey({ autoIncrement: true }),
    // Epoch millis, defaulted in JS: see tool-dispatches.sql.ts. A SQL
    // `unixepoch('subsec')` default needs SQLite >= 3.42, which the
    // macOS system SQLite behind bun:sqlite may predate, yielding NULL
    // and a NOT NULL failure on every insert. `Date.now()` is portable.
    createdAt: integer('created_at')
      .notNull()
      .$defaultFn(() => Date.now()),
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
