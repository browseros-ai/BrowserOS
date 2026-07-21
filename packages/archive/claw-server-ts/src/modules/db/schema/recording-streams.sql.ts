/** Document-keyed rrweb stream catalog; target identity is optional metadata. */

import { index, integer, sqliteTable, text } from 'drizzle-orm/sqlite-core'

export const recordingStreams = sqliteTable(
  'recording_streams',
  {
    documentId: text('document_id').primaryKey(),
    tabId: integer('tab_id').notNull(),
    targetId: text('target_id'),
    firstEventAt: integer('first_event_at').notNull(),
    lastEventAt: integer('last_event_at').notNull(),
    sizeBytes: integer('size_bytes').notNull(),
    eventCount: integer('event_count').notNull(),
    hasGap: integer('has_gap', { mode: 'boolean' }).notNull().default(false),
  },
  (table) => [
    index('recording_streams_tab_time_idx').on(
      table.tabId,
      table.firstEventAt,
      table.lastEventAt,
    ),
    index('recording_streams_retention_idx').on(table.lastEventAt),
  ],
)

export type RecordingStreamRow = typeof recordingStreams.$inferSelect
export type NewRecordingStream = typeof recordingStreams.$inferInsert
