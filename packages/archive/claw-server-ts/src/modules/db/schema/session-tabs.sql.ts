/**
 * Server-owned attribution windows for logical Chrome tabs. `openedTargetId`
 * records the target observed at claim time; replay joins on `tabId` because a
 * cross-process navigation can replace the target without replacing the tab.
 */

import { sql } from 'drizzle-orm'
import {
  index,
  integer,
  sqliteTable,
  text,
  uniqueIndex,
} from 'drizzle-orm/sqlite-core'

export const sessionTabs = sqliteTable(
  'session_tabs',
  {
    id: integer('id').primaryKey({ autoIncrement: true }),
    sessionId: text('session_id').notNull(),
    agentId: text('agent_id').notNull(),
    tabId: integer('tab_id').notNull(),
    openedTargetId: text('opened_target_id'),
    claimedAt: integer('claimed_at').notNull(),
    releasedAt: integer('released_at'),
  },
  (table) => [
    index('session_tabs_session_idx').on(table.sessionId, table.claimedAt),
    index('session_tabs_tab_window_idx').on(
      table.tabId,
      table.claimedAt,
      table.releasedAt,
    ),
    uniqueIndex('session_tabs_one_live_owner_idx')
      .on(table.tabId)
      .where(sql`${table.releasedAt} is null`),
  ],
)

export type SessionTabRow = typeof sessionTabs.$inferSelect
export type NewSessionTab = typeof sessionTabs.$inferInsert
