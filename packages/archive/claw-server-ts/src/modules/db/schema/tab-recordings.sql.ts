/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

/**
 * Catalog for one target-keyed NDJSON file per row. Event bounds are Unix epoch
 * milliseconds; counts and bytes describe the whole file, not a claim window.
 */
export const tabRecordings = sqliteTable(
  'tab_recordings',
  {
    targetId: text('target_id').primaryKey(),
    tabId: integer('tab_id').notNull(),
    firstEventAt: integer('first_event_at').notNull(),
    lastEventAt: integer('last_event_at').notNull(),
    sizeBytes: integer('size_bytes').notNull(),
    eventCount: integer('event_count').notNull(),
  },
  (table) => [index('tab_recordings_last_event_idx').on(table.lastEventAt)],
)

export type TabRecordingRow = typeof tabRecordings.$inferSelect
export type NewTabRecording = typeof tabRecordings.$inferInsert
