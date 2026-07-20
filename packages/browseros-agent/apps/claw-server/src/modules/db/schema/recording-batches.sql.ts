/** Durable acceptance ledger for extension retries across server restarts. */

import { integer, primaryKey, sqliteTable, text } from 'drizzle-orm/sqlite-core'
import { recordingStreams } from './recording-streams.sql'

export const recordingBatches = sqliteTable(
  'recording_batches',
  {
    documentId: text('document_id')
      .notNull()
      .references(() => recordingStreams.documentId, { onDelete: 'cascade' }),
    batchId: text('batch_id').notNull(),
    acceptedAt: integer('accepted_at').notNull(),
  },
  (table) => [primaryKey({ columns: [table.documentId, table.batchId] })],
)

export type RecordingBatchRow = typeof recordingBatches.$inferSelect
