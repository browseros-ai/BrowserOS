import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { eq } from 'drizzle-orm'
import {
  getAuditDb,
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../src/modules/db/db'
import { recordingBatches } from '../../src/modules/db/schema/recording-batches.sql'
import { recordingPayloads } from '../../src/modules/db/schema/recording-payloads.sql'
import { recordingStreams } from '../../src/modules/db/schema/recording-streams.sql'
import { sessionTabs } from '../../src/modules/db/schema/session-tabs.sql'
import {
  createRecordingStore,
  RECORDING_ORPHAN_TTL_MS,
  type RecordingStore,
} from '../../src/services/recordings'

const documentA = '33D25F3CF060E81B14070BC356FF1871'
const documentB = '8395FF2EF4A1D8579F1917B3B54ADECE'
let dir: string
let store: RecordingStore

beforeEach(async () => {
  dir = await mkdtemp(join(tmpdir(), 'recordings-'))
  setAuditDbForTesting()
  store = createRecordingStore({ rootDir: dir })
})

afterEach(async () => {
  await store.resetForTesting()
  resetAuditDbForTesting()
  await rm(dir, { recursive: true, force: true })
})

function append(
  documentId: string,
  tabId: number,
  batchId: string,
  timestamps: number[],
  overrides: { targetId?: string | null; hasGap?: boolean } = {},
) {
  return store.appendBatch({
    documentId,
    tabId,
    targetId: overrides.targetId ?? null,
    events: timestamps.map((ts) => ({ ts, type: 3, data: { ts } })),
    batchId,
    hasGap: overrides.hasGap ?? false,
  })
}

describe('RecordingStore', () => {
  it('appends document-keyed events and catalogs optional target metadata', async () => {
    await append(documentA, 11, 'batch-a', [200, 100])
    await append(documentA, 11, 'batch-b', [300], {
      targetId: 'target-after-resolution',
    })

    const events = [
      { ts: 200, type: 3, data: { ts: 200 } },
      { ts: 100, type: 3, data: { ts: 100 } },
      { ts: 300, type: 3, data: { ts: 300 } },
    ]
    expect(await store.readRange(documentA, 0, 400)).toEqual(events)
    const eventsNdjson = `${events.map(JSON.stringify).join('\n')}\n`
    expect(
      getAuditDb()
        .select()
        .from(recordingStreams)
        .where(eq(recordingStreams.documentId, documentA))
        .get(),
    ).toEqual({
      documentId: documentA,
      tabId: 11,
      targetId: 'target-after-resolution',
      firstEventAt: 100,
      lastEventAt: 300,
      sizeBytes: Buffer.byteLength(eventsNdjson),
      eventCount: 3,
      hasGap: false,
    })
    expect(
      getAuditDb()
        .select()
        .from(recordingPayloads)
        .where(eq(recordingPayloads.documentId, documentA))
        .get(),
    ).toEqual({ documentId: documentA, eventsNdjson })
  })

  it('reads only events inside an inclusive ownership window', async () => {
    await append(documentA, 11, 'batch-a', [100, 200, 300])
    expect(await store.readRange(documentA, 100, 200)).toEqual([
      { ts: 100, type: 3, data: { ts: 100 } },
      { ts: 200, type: 3, data: { ts: 200 } },
    ])
  })

  it('deduplicates accepted batch ids after the store is recreated', async () => {
    expect(await append(documentA, 11, 'batch-a', [100])).toBe(true)
    await store.close()
    store = createRecordingStore({ rootDir: dir })

    expect(await append(documentA, 11, 'batch-a', [100])).toBe(false)
    expect(await append(documentB, 11, 'batch-a', [100])).toBe(true)
    expect(
      getAuditDb()
        .select()
        .from(recordingBatches)
        .all()
        .map((row) => [row.documentId, row.batchId]),
    ).toEqual([
      [documentA, 'batch-a'],
      [documentB, 'batch-a'],
    ])
    expect(
      getAuditDb()
        .select({ documentId: recordingPayloads.documentId })
        .from(recordingPayloads)
        .all(),
    ).toEqual([{ documentId: documentA }, { documentId: documentB }])
  })

  it('serializes concurrent retries before the durable batch check', async () => {
    const results = await Promise.all([
      append(documentA, 11, 'batch-a', [100]),
      append(documentA, 11, 'batch-a', [100]),
    ])
    expect(results.sort()).toEqual([false, true])
    expect(await store.readRange(documentA, 0, 200)).toHaveLength(1)
  })

  it('rejects a document id that moves to another tab', async () => {
    await append(documentA, 11, 'batch-a', [100])

    await expect(append(documentA, 12, 'batch-b', [200])).rejects.toThrow(
      `recording document ${documentA} changed tab identity`,
    )
    expect(await store.readRange(documentA, 0, 300)).toEqual([
      { ts: 100, type: 3, data: { ts: 100 } },
    ])
  })

  it('keeps a gap sticky across later complete batches', async () => {
    await append(documentA, 11, 'batch-a', [100], { hasGap: true })
    await append(documentA, 11, 'batch-b', [200])
    expect(
      getAuditDb()
        .select({ hasGap: recordingStreams.hasGap })
        .from(recordingStreams)
        .where(eq(recordingStreams.documentId, documentA))
        .get(),
    ).toEqual({ hasGap: true })
  })

  it('uses orphan TTL only for streams without an overlapping claim', async () => {
    const now = 10 * RECORDING_ORPHAN_TTL_MS
    const normalRetentionMs = 7 * 24 * 60 * 60 * 1000
    await append(documentA, 11, 'claimed', [now - 2 * RECORDING_ORPHAN_TTL_MS])
    await append(documentB, 22, 'orphan', [now - 2 * RECORDING_ORPHAN_TTL_MS])
    getAuditDb()
      .insert(sessionTabs)
      .values({
        sessionId: 'session-a',
        agentId: 'agent-a',
        tabId: 11,
        openedTargetId: null,
        claimedAt: now - normalRetentionMs,
        releasedAt: now,
      })
      .run()

    expect(await store.sweepRetention(7, now)).toEqual({
      recordingsDeleted: 1,
      claimsDeleted: 0,
    })
    expect(
      getAuditDb()
        .select({ documentId: recordingStreams.documentId })
        .from(recordingStreams)
        .all(),
    ).toEqual([{ documentId: documentA }])
    expect(
      getAuditDb()
        .select()
        .from(recordingBatches)
        .where(eq(recordingBatches.documentId, documentB))
        .all(),
    ).toEqual([])
    expect(
      getAuditDb()
        .select()
        .from(recordingPayloads)
        .where(eq(recordingPayloads.documentId, documentB))
        .all(),
    ).toEqual([])
  })
})
