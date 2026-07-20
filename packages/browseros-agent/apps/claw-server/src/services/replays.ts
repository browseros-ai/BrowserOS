/** Read-time join from server-owned tab windows to document recording streams. */

import { and, eq, gte, lte, sql } from 'drizzle-orm'
import { type AuditDb, getAuditDb } from '../modules/db/db'
import { recordingStreams } from '../modules/db/schema/recording-streams.sql'
import { sessionTabs } from '../modules/db/schema/session-tabs.sql'
import { tabClaims } from '../modules/db/schema/tab-claims.sql'
import { tabRecordings } from '../modules/db/schema/tab-recordings.sql'
import {
  legacyDocumentId,
  type RecordedEvent,
  type RecordingStore,
  recordingStore,
} from './recordings'

export interface ReplayEvent extends RecordedEvent {
  sessionId: string
  documentId: string
  tabId: number
  targetId: string | null
}

export interface ReplaySegmentMeta {
  documentId: string
  targetId: string | null
  firstEventAt: number
  lastEventAt: number
  sizeBytes: number
  eventCount: number
  hasGap: boolean
  legacy: boolean
}

export interface ReplayTabMeta {
  tabId: number
  complete: boolean
  firstEventAt: number
  lastEventAt: number
  segments: ReplaySegmentMeta[]
}

export interface ReplayMeta {
  exists: boolean
  complete: boolean
  firstEventAt?: number
  lastEventAt?: number
  sizeBytes: number
  tabs: ReplayTabMeta[]
}

export interface ReplayService {
  readSession(sessionId: string): Promise<ReplayEvent[]>
  getMeta(sessionId: string): ReplayMeta
}

interface ReplayServiceOptions {
  recordingStore: RecordingStore
  getDb?: () => AuditDb
}

/** Selects persisted streams by tab/time overlap and filters each event exactly. */
export function createReplayService(
  options: ReplayServiceOptions,
): ReplayService {
  const getDb = options.getDb ?? getAuditDb

  return {
    async readSession(sessionId) {
      const matches = matchingStreams(getDb(), sessionId)
      const streams = groupMatches(matches)
      const slices = await Promise.all(
        streams.map(async ({ stream, windows: tabWindows }) => {
          const from = Math.min(...tabWindows.map((window) => window.claimedAt))
          const to = Math.max(
            ...tabWindows.map(
              (window) => window.releasedAt ?? Number.MAX_SAFE_INTEGER,
            ),
          )
          return (
            await options.recordingStore.readRange(stream.documentId, from, to)
          )
            .filter((event) => eventInWindows(event.ts, tabWindows))
            .map((event) => ({
              sessionId,
              documentId: stream.documentId,
              tabId: stream.tabId,
              targetId: stream.targetId,
              ...event,
            }))
        }),
      )
      const legacy = await readLegacySession(
        getDb(),
        options.recordingStore,
        sessionId,
      )
      return [...slices.flat(), ...legacy].sort(
        (left, right) => left.ts - right.ts,
      )
    },
    getMeta(sessionId) {
      const segments = groupMatches(matchingStreams(getDb(), sessionId)).map(
        ({
          stream,
          windows: tabWindows,
        }): {
          tabId: number
          segment: ReplaySegmentMeta
        } => {
          return {
            tabId: stream.tabId,
            segment: {
              documentId: stream.documentId,
              targetId: stream.targetId,
              firstEventAt: Math.max(
                stream.firstEventAt,
                Math.min(...tabWindows.map((window) => window.claimedAt)),
              ),
              lastEventAt: Math.min(
                stream.lastEventAt,
                Math.max(
                  ...tabWindows.map(
                    (window) => window.releasedAt ?? Number.MAX_SAFE_INTEGER,
                  ),
                ),
              ),
              sizeBytes: stream.sizeBytes,
              eventCount: stream.eventCount,
              hasGap: stream.hasGap,
              legacy: stream.documentId.startsWith('legacy-'),
            },
          }
        },
      )
      segments.push(...legacyMeta(getDb(), sessionId))
      return buildMeta(segments)
    },
  }
}

type Window = typeof sessionTabs.$inferSelect
type Stream = typeof recordingStreams.$inferSelect

interface StreamMatch {
  stream: Stream
  window: Window
}

function matchingStreams(db: AuditDb, sessionId: string): StreamMatch[] {
  return db
    .select({ stream: recordingStreams, window: sessionTabs })
    .from(sessionTabs)
    .innerJoin(
      recordingStreams,
      and(
        eq(recordingStreams.tabId, sessionTabs.tabId),
        gte(recordingStreams.lastEventAt, sessionTabs.claimedAt),
        lte(
          recordingStreams.firstEventAt,
          sql`coalesce(${sessionTabs.releasedAt}, 9223372036854775807)`,
        ),
      ),
    )
    .where(eq(sessionTabs.sessionId, sessionId))
    .orderBy(recordingStreams.firstEventAt)
    .all()
}

function groupMatches(
  matches: StreamMatch[],
): Array<{ stream: Stream; windows: Window[] }> {
  const grouped = new Map<string, { stream: Stream; windows: Window[] }>()
  for (const { stream, window } of matches) {
    const entry = grouped.get(stream.documentId) ?? { stream, windows: [] }
    entry.windows.push(window)
    grouped.set(stream.documentId, entry)
  }
  return [...grouped.values()]
}

function eventInWindows(timestamp: number, windows: Window[]): boolean {
  return windows.some(
    (window) =>
      timestamp >= window.claimedAt &&
      timestamp <= (window.releasedAt ?? Number.MAX_SAFE_INTEGER),
  )
}

async function readLegacySession(
  db: AuditDb,
  store: RecordingStore,
  sessionId: string,
): Promise<ReplayEvent[]> {
  const claims = db
    .select()
    .from(tabClaims)
    .where(eq(tabClaims.sessionId, sessionId))
    .all()
  const slices = await Promise.all(
    claims.map(async (claim) => {
      const events = await store.readLegacyRange(
        claim.targetId,
        claim.claimedAt,
        claim.releasedAt ?? Number.MAX_SAFE_INTEGER,
      )
      return events.map(({ tabId, ...event }) => ({
        sessionId,
        documentId: legacyDocumentId(claim.targetId),
        tabId,
        targetId: claim.targetId,
        ...event,
      }))
    }),
  )
  return slices.flat()
}

function legacyMeta(
  db: AuditDb,
  sessionId: string,
): Array<{ tabId: number; segment: ReplaySegmentMeta }> {
  const claims = db
    .select()
    .from(tabClaims)
    .where(eq(tabClaims.sessionId, sessionId))
    .all()
  const recordings = new Map(
    db
      .select()
      .from(tabRecordings)
      .all()
      .map((recording) => [recording.targetId, recording]),
  )
  return claims.flatMap((claim) => {
    const recording = recordings.get(claim.targetId)
    if (!recording) return []
    const firstEventAt = Math.max(claim.claimedAt, recording.firstEventAt)
    const lastEventAt = Math.min(
      claim.releasedAt ?? Number.MAX_SAFE_INTEGER,
      recording.lastEventAt,
    )
    if (firstEventAt > lastEventAt) return []
    return [
      {
        tabId: recording.tabId,
        segment: {
          documentId: legacyDocumentId(claim.targetId),
          targetId: claim.targetId,
          firstEventAt,
          lastEventAt,
          sizeBytes: recording.sizeBytes,
          eventCount: recording.eventCount,
          hasGap: true,
          legacy: true,
        },
      },
    ]
  })
}

function buildMeta(
  entries: Array<{ tabId: number; segment: ReplaySegmentMeta }>,
): ReplayMeta {
  if (entries.length === 0) return emptyMeta()
  const byTab = new Map<number, ReplaySegmentMeta[]>()
  for (const { tabId, segment } of entries) {
    const segments = byTab.get(tabId) ?? []
    if (
      !segments.some((candidate) => candidate.documentId === segment.documentId)
    ) {
      segments.push(segment)
    }
    byTab.set(tabId, segments)
  }
  const tabs = [...byTab].map(([tabId, segments]): ReplayTabMeta => {
    segments.sort((left, right) => left.firstEventAt - right.firstEventAt)
    return {
      tabId,
      complete: segments.every((segment) => !segment.hasGap && !segment.legacy),
      firstEventAt: Math.min(
        ...segments.map((segment) => segment.firstEventAt),
      ),
      lastEventAt: Math.max(...segments.map((segment) => segment.lastEventAt)),
      segments,
    }
  })
  tabs.sort((left, right) => left.firstEventAt - right.firstEventAt)
  return {
    exists: true,
    complete: tabs.every((tab) => tab.complete),
    firstEventAt: Math.min(...tabs.map((tab) => tab.firstEventAt)),
    lastEventAt: Math.max(...tabs.map((tab) => tab.lastEventAt)),
    sizeBytes: tabs.reduce(
      (sum, tab) =>
        sum +
        tab.segments.reduce((tabSum, segment) => tabSum + segment.sizeBytes, 0),
      0,
    ),
    tabs,
  }
}

function emptyMeta(): ReplayMeta {
  return { exists: false, complete: true, sizeBytes: 0, tabs: [] }
}

export const replayService = createReplayService({ recordingStore })
