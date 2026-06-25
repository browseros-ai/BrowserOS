/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * One row per MCP session handshake: written from the server's
 * `oninitialized` hook after clientInfo lands. Captures who started
 * the session and at what wall clock. Lets the task deriver attach
 * a real start moment instead of inferring from the first dispatch.
 */

import { sql } from 'drizzle-orm'
import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

export const agentSessionStarts = sqliteTable(
  'agent_session_starts',
  {
    id: integer('id').primaryKey({ autoIncrement: true }),
    createdAt: integer('created_at')
      .notNull()
      .default(sql`(unixepoch('subsec') * 1000)`),
    sessionId: text('session_id').notNull(),
    agentId: text('agent_id').notNull(),
    slug: text('slug').notNull(),
    agentLabel: text('agent_label').notNull(),
    clientName: text('client_name').notNull(),
    clientVersion: text('client_version').notNull(),
  },
  (t) => ({
    sessionIdx: index('agent_session_starts_session_idx').on(t.sessionId),
    createdAtIdx: index('agent_session_starts_created_at_idx').on(t.createdAt),
  }),
)

export type AgentSessionStartRow = typeof agentSessionStarts.$inferSelect
export type NewAgentSessionStart = typeof agentSessionStarts.$inferInsert
