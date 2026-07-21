/** Raw rrweb NDJSON stored separately so metadata joins stay lightweight. */

import { sqliteTable, text } from 'drizzle-orm/sqlite-core'
import { recordingStreams } from './recording-streams.sql'

export const recordingPayloads = sqliteTable('recording_payloads', {
  documentId: text('document_id')
    .primaryKey()
    .references(() => recordingStreams.documentId, { onDelete: 'cascade' }),
  eventsNdjson: text('events_ndjson').notNull(),
})

export type RecordingPayloadRow = typeof recordingPayloads.$inferSelect
