/**
 * Document-keyed rrweb persistence. Recorder identity and session attribution
 * are intentionally separate: this store accepts Chrome document streams now,
 * while `replays.ts` joins them to server-owned tab windows later.
 */

import { readFile, rm, unlink } from 'node:fs/promises'
import { join } from 'node:path'
import { and, eq, isNotNull, lt, sql } from 'drizzle-orm'
import { resolveClawServerPath } from '../lib/browserclaw-dir'
import { logger } from '../lib/logger'
import { type AuditDb, getAuditDb } from '../modules/db/db'
import { recordingBatches } from '../modules/db/schema/recording-batches.sql'
import { recordingPayloads } from '../modules/db/schema/recording-payloads.sql'
import { recordingStreams } from '../modules/db/schema/recording-streams.sql'
import { sessionTabs } from '../modules/db/schema/session-tabs.sql'
import { tabClaims } from '../modules/db/schema/tab-claims.sql'
import { tabRecordings } from '../modules/db/schema/tab-recordings.sql'

const RECORDINGS_DIR_NAME = 'recordings'
const RETENTION_INTERVAL_MS = 60 * 60 * 1000
const DAY_MS = 24 * 60 * 60 * 1000
export const RECORDING_ORPHAN_TTL_MS = 60 * 60 * 1000

export interface RecordingEventInput {
  ts: number
  type: unknown
  data: unknown
}

export interface AppendRecordingBatchInput {
  documentId: string
  tabId: number
  targetId: string | null
  events: RecordingEventInput[]
  batchId: string
  hasGap: boolean
}

export interface RecordedEvent extends RecordingEventInput {}

export interface LegacyRecordedEvent extends RecordingEventInput {
  tabId: number
}

export interface RetentionSweepResult {
  recordingsDeleted: number
  claimsDeleted: number
}

export interface RecordingStore {
  /** Returns false only when this document already durably accepted the batch. */
  appendBatch(input: AppendRecordingBatchInput): Promise<boolean>
  readRange(
    documentId: string,
    from: number,
    to: number,
  ): Promise<RecordedEvent[]>
  readLegacyRange(
    targetId: string,
    from: number,
    to: number,
  ): Promise<LegacyRecordedEvent[]>
  sweepRetention(
    retentionDays: number,
    now?: number,
  ): Promise<RetentionSweepResult>
  close(): Promise<void>
  resetForTesting(): Promise<void>
}

export interface RecordingStoreOptions {
  rootDir?: string
  getDb?: () => AuditDb
  now?: () => number
}

/** Stores document streams and their batch ledger in one SQLite transaction. */
export function createRecordingStore(
  options: RecordingStoreOptions = {},
): RecordingStore {
  const getDb = options.getDb ?? getAuditDb
  const now = options.now ?? Date.now
  const chains = new Map<string, Promise<unknown>>()

  function resolvePath(documentId: string): string {
    const root = options.rootDir ?? resolveClawServerPath(RECORDINGS_DIR_NAME)
    return join(root, `${sanitizeRecordingId(documentId)}.ndjson`)
  }

  function enqueue<T>(
    documentId: string,
    operation: () => Promise<T>,
  ): Promise<T> {
    const previous = chains.get(documentId) ?? Promise.resolve()
    const next = previous.catch(() => undefined).then(operation)
    const tracked = next.finally(() => {
      if (chains.get(documentId) === tracked) chains.delete(documentId)
    })
    chains.set(documentId, tracked)
    return tracked
  }

  async function append(input: AppendRecordingBatchInput): Promise<boolean> {
    if (
      getDb()
        .select({ batchId: recordingBatches.batchId })
        .from(recordingBatches)
        .where(
          and(
            eq(recordingBatches.documentId, input.documentId),
            eq(recordingBatches.batchId, input.batchId),
          ),
        )
        .get()
    ) {
      return false
    }
    const existingStream = getDb()
      .select({ tabId: recordingStreams.tabId })
      .from(recordingStreams)
      .where(eq(recordingStreams.documentId, input.documentId))
      .get()
    if (existingStream && existingStream.tabId !== input.tabId) {
      throw new Error(
        `recording document ${input.documentId} changed tab identity`,
      )
    }
    if (input.events.length === 0) return true

    const lines = input.events.map((event) => JSON.stringify(event))
    const payload = `${lines.join('\n')}\n`
    const firstEventAt = Math.min(...input.events.map((event) => event.ts))
    const lastEventAt = Math.max(...input.events.map((event) => event.ts))
    const sizeBytes = Buffer.byteLength(payload)
    getDb().transaction((tx) => {
      tx.insert(recordingStreams)
        .values({
          documentId: input.documentId,
          tabId: input.tabId,
          targetId: input.targetId,
          firstEventAt,
          lastEventAt,
          sizeBytes,
          eventCount: input.events.length,
          hasGap: input.hasGap,
        })
        .onConflictDoUpdate({
          target: recordingStreams.documentId,
          set: {
            targetId: sql`coalesce(${recordingStreams.targetId}, ${input.targetId})`,
            firstEventAt: sql`min(${recordingStreams.firstEventAt}, ${firstEventAt})`,
            lastEventAt: sql`max(${recordingStreams.lastEventAt}, ${lastEventAt})`,
            sizeBytes: sql`${recordingStreams.sizeBytes} + ${sizeBytes}`,
            eventCount: sql`${recordingStreams.eventCount} + ${input.events.length}`,
            hasGap: sql`${recordingStreams.hasGap} or ${input.hasGap ? 1 : 0}`,
          },
        })
        .run()
      tx.insert(recordingPayloads)
        .values({ documentId: input.documentId, eventsNdjson: payload })
        .onConflictDoUpdate({
          target: recordingPayloads.documentId,
          set: {
            eventsNdjson: sql`${recordingPayloads.eventsNdjson} || ${payload}`,
          },
        })
        .run()
      tx.insert(recordingBatches)
        .values({
          documentId: input.documentId,
          batchId: input.batchId,
          acceptedAt: now(),
        })
        .run()
    })
    return true
  }

  async function readDocumentRange(
    documentId: string,
    from: number,
    to: number,
  ): Promise<RecordedEvent[]> {
    await chains.get(documentId)?.catch(() => undefined)
    const text =
      getDb()
        .select({ eventsNdjson: recordingPayloads.eventsNdjson })
        .from(recordingPayloads)
        .where(eq(recordingPayloads.documentId, documentId))
        .get()?.eventsNdjson ?? ''
    const events: RecordedEvent[] = []
    for (const line of text.split('\n')) {
      if (!line) continue
      const event = parseRecordedEvent(line)
      if (event && event.ts >= from && event.ts <= to) events.push(event)
    }
    return events
  }

  async function deleteDocument(documentId: string): Promise<boolean> {
    return enqueue(documentId, async () => {
      const row = getDb()
        .select({ documentId: recordingStreams.documentId })
        .from(recordingStreams)
        .where(eq(recordingStreams.documentId, documentId))
        .get()
      if (!row) return false
      getDb()
        .delete(recordingStreams)
        .where(eq(recordingStreams.documentId, documentId))
        .run()
      return true
    })
  }

  async function deleteLegacyTarget(
    targetId: string,
    cutoff: number,
  ): Promise<boolean> {
    return enqueue(`legacy-file:${targetId}`, async () => {
      const row = getDb()
        .select({ lastEventAt: tabRecordings.lastEventAt })
        .from(tabRecordings)
        .where(eq(tabRecordings.targetId, targetId))
        .get()
      if (!row || row.lastEventAt >= cutoff) return false
      if (!(await removeRecordingFile(resolvePath(targetId), { targetId }))) {
        return false
      }
      getDb()
        .delete(tabRecordings)
        .where(eq(tabRecordings.targetId, targetId))
        .run()
      return true
    })
  }

  return {
    appendBatch(input) {
      return enqueue(input.documentId, () => append(input))
    },
    readRange: readDocumentRange,
    async readLegacyRange(targetId, from, to) {
      await chains.get(`legacy-file:${targetId}`)?.catch(() => undefined)
      let text: string
      try {
        text = await readFile(resolvePath(targetId), 'utf8')
      } catch (error) {
        if ((error as { code?: string }).code === 'ENOENT') return []
        throw error
      }
      const events: LegacyRecordedEvent[] = []
      for (const line of text.split('\n')) {
        if (!line) continue
        const event = parseLegacyRecordedEvent(line)
        if (event && event.ts >= from && event.ts <= to) events.push(event)
      }
      return events
    },
    async sweepRetention(retentionDays, timestamp = now()) {
      const retentionCutoff = timestamp - retentionDays * DAY_MS
      const orphanCutoff = timestamp - RECORDING_ORPHAN_TTL_MS
      const claims = getDb().select().from(sessionTabs).all()
      const streams = getDb().select().from(recordingStreams).all()
      let recordingsDeleted = 0

      for (const stream of streams) {
        const claimed = claims.some(
          (claim) =>
            claim.tabId === stream.tabId &&
            stream.lastEventAt >= claim.claimedAt &&
            stream.firstEventAt <=
              (claim.releasedAt ?? Number.MAX_SAFE_INTEGER),
        )
        const cutoff = claimed ? retentionCutoff : orphanCutoff
        if (
          stream.lastEventAt < cutoff &&
          (await deleteDocument(stream.documentId))
        ) {
          recordingsDeleted++
        }
      }

      const legacyExpired = getDb()
        .select({ targetId: tabRecordings.targetId })
        .from(tabRecordings)
        .where(lt(tabRecordings.lastEventAt, retentionCutoff))
        .all()
      for (const { targetId } of legacyExpired) {
        if (await deleteLegacyTarget(targetId, retentionCutoff)) {
          recordingsDeleted++
        }
      }

      const oldSessionTabIds = getDb()
        .select({ id: sessionTabs.id })
        .from(sessionTabs)
        .where(
          and(
            isNotNull(sessionTabs.releasedAt),
            lt(sessionTabs.releasedAt, retentionCutoff),
          ),
        )
        .all()
      const oldTargetClaimIds = getDb()
        .select({ id: tabClaims.id })
        .from(tabClaims)
        .where(
          and(
            isNotNull(tabClaims.releasedAt),
            lt(tabClaims.releasedAt, retentionCutoff),
          ),
        )
        .all()
      getDb()
        .delete(sessionTabs)
        .where(
          and(
            isNotNull(sessionTabs.releasedAt),
            lt(sessionTabs.releasedAt, retentionCutoff),
          ),
        )
        .run()
      getDb()
        .delete(tabClaims)
        .where(
          and(
            isNotNull(tabClaims.releasedAt),
            lt(tabClaims.releasedAt, retentionCutoff),
          ),
        )
        .run()
      return {
        recordingsDeleted,
        claimsDeleted: oldSessionTabIds.length + oldTargetClaimIds.length,
      }
    },
    async close() {
      while (chains.size > 0) {
        await Promise.allSettled([...chains.values()])
      }
    },
    async resetForTesting() {
      await this.close()
      if (options.rootDir) {
        await rm(options.rootDir, { recursive: true, force: true })
      }
    },
  }
}

async function removeRecordingFile(
  path: string,
  fields: Record<string, unknown>,
): Promise<boolean> {
  try {
    await unlink(path)
    return true
  } catch (error) {
    if ((error as { code?: string }).code === 'ENOENT') return true
    logger.warn('recording retention unlink failed', {
      ...fields,
      error: error instanceof Error ? error.message : String(error),
    })
    return false
  }
}

export interface RecordingRetentionHandle {
  initialSweep: Promise<void>
  stop(): Promise<void>
}

/** Runs recording retention at startup and hourly without keeping Bun alive. */
export function startRecordingRetention(
  store: RecordingStore,
  retentionDays: number,
  intervalMs = RETENTION_INTERVAL_MS,
): RecordingRetentionHandle {
  const activeSweeps = new Set<Promise<void>>()
  const run = async (): Promise<void> => {
    try {
      const result = await store.sweepRetention(retentionDays)
      logger.info('recording retention sweep finished', { ...result })
    } catch (error) {
      logger.warn('recording retention sweep failed', {
        error: error instanceof Error ? error.message : String(error),
      })
    }
  }
  const launch = (): Promise<void> => {
    const sweep = run().finally(() => activeSweeps.delete(sweep))
    activeSweeps.add(sweep)
    return sweep
  }
  const initialSweep = launch()
  const timer = setInterval(() => void launch(), intervalMs)
  timer.unref?.()
  return {
    initialSweep,
    stop: async () => {
      clearInterval(timer)
      await Promise.allSettled([...activeSweeps])
    },
  }
}

export function legacyDocumentId(targetId: string): string {
  return `legacy-${targetId}`
}

function sanitizeRecordingId(id: string): string {
  return id.replace(/[^A-Za-z0-9._-]/g, '_')
}

function parseRecordedEvent(line: string): RecordedEvent | null {
  try {
    const event = JSON.parse(line) as Partial<RecordingEventInput>
    if (typeof event.ts !== 'number') return null
    return { ts: event.ts, type: event.type, data: event.data }
  } catch {
    return null
  }
}

function parseLegacyRecordedEvent(line: string): LegacyRecordedEvent | null {
  try {
    const event = JSON.parse(line) as Partial<LegacyRecordedEvent>
    if (typeof event.tabId !== 'number' || typeof event.ts !== 'number') {
      return null
    }
    return {
      tabId: event.tabId,
      ts: event.ts,
      type: event.type,
      data: event.data,
    }
  } catch {
    return null
  }
}

export const recordingStore = createRecordingStore()
