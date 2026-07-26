/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

/**
 * Epoch-millisecond attribution windows from server-owned MCP sessions to CDP
 * targets. `releasedAt` stays null while the session still controls the target.
 */
export const tabClaims = sqliteTable(
  'tab_claims',
  {
    id: integer('id').primaryKey({ autoIncrement: true }),
    targetId: text('target_id').notNull(),
    sessionId: text('session_id').notNull(),
    agentId: text('agent_id').notNull(),
    claimedAt: integer('claimed_at').notNull(),
    releasedAt: integer('released_at'),
  },
  (table) => [
    index('tab_claims_target_idx').on(table.targetId, table.claimedAt),
    index('tab_claims_session_idx').on(table.sessionId),
  ],
)

export type TabClaimRow = typeof tabClaims.$inferSelect
export type NewTabClaim = typeof tabClaims.$inferInsert
