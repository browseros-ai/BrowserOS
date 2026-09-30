/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Session-replay API surface for the claw-app cockpit.
 * Metadata polling lets audit views discover newly available recordings without
 * repeatedly downloading the session-keyed NDJSON event snapshot.
 */

import type { RecordingMetadata } from '@browseros/claw-api'
import { ApiResponseError } from '@browseros/claw-api-client'
import { createQuery } from 'react-query-kit'
import { apiClient } from './client'

export type ReplayVerb =
  | 'navigate'
  | 'read'
  | 'click'
  | 'type'
  | 'attach'
  | 'submit'
  | 'done'

export type ReplayKind = 'action' | 'block' | 'done'

export interface ReplayFrame {
  /** Seconds into the session, at which this dispatch completed. */
  t: number
  /**
   * How long the tool ran, from the source dispatch row. `session-replay.ts`
   * subtracts it from `t` to approximate when the action began, which is when
   * the replay starts treating it as current. Absent on rows recorded before
   * durations were persisted; negative and non-finite values fall back to the
   * completion time.
   */
  durationMs?: number | null
  kind: ReplayKind
  verb: ReplayVerb
  /** Short node label, e.g. the page title or a focused element. */
  node: string
  /** Caption sentence rendered in the viewport overlay + timeline row. */
  caption: string
  /**
   * Full URL captured on this dispatch's audit row, when the tool
   * targeted a page. Populates the replay viewport's browser-chrome
   * address bar so the operator can see the exact URL the agent
   * was on at this instant. Null for tools that do not target a
   * page (`run`, `windows`, `tab_groups`, `tabs new` before the
   * result comes back).
   */
  url?: string | null
  pageId?: number | null
  /** Chrome tab that owned this dispatch, when known. */
  tabId?: number | null
  /** CDP target observed for this dispatch; may change across navigation. */
  targetId?: string | null
  /** Optional badge shown on the timeline row ("Blocked", "Cancelled"). */
  note?: string
  /** Source dispatch id so the replay surface can deep-link. */
  dispatchId?: number
}

export interface ReplayEvent {
  /** MCP session attributed from persisted claim state, not recorder input. */
  sessionId: string
  /** Chrome document stream; a new value marks a navigation boundary. */
  documentId: string
  /** Best-effort CDP metadata observed when this document was recorded. */
  targetId: string | null
  /** Chrome tab id captured at ingest; distinct from a BrowserOS page id. */
  tabId: number
  type: number
  data: unknown
  /** rrweb event timestamp in Unix epoch milliseconds. */
  ts: number
}

export type ReplayMetadata = RecordingMetadata

export interface UseReplayMetadataVariables {
  sessionId: string
}

/** Cheap metadata probe behind the "View Replay" CTA and page picker. */
export async function fetchReplayMetadata({
  sessionId,
}: UseReplayMetadataVariables): Promise<ReplayMetadata> {
  return (await apiClient()).getRecording({ sessionId })
}

// No factory refetchInterval: callers opt in to polling (see
// replayMetadataRefetchInterval), so a finished session fetches the recording
// metadata once and stops.
export const useReplayMetadata = createQuery<
  ReplayMetadata,
  UseReplayMetadataVariables
>({
  queryKey: ['replay', 'metadata'],
  fetcher: fetchReplayMetadata,
})

export const REPLAY_METADATA_POLL_MS = 10_000
// Recording batches upload asynchronously and a final batch can land shortly
// after a session goes terminal. Keep polling for a bounded window after the
// end so that trailing batch (and the View Replay unlock) is still picked up,
// without polling forever on a session that never records. `complete` is NOT
// used as the stop signal: per the API it flags segment gaps, not finalisation,
// so it can stay false permanently.
const REPLAY_FINALIZE_GRACE_MS = 30_000

/**
 * Poll interval for recording metadata, or false to stop. Polls while the
 * session is live, and briefly after it ends until the recording first appears
 * (`hasData`) or the grace window from `endedAt` elapses.
 */
export function replayMetadataRefetchInterval(args: {
  status: string | undefined
  endedAt: number | null | undefined
  hasData: boolean | undefined
  now?: number
}): number | false {
  if (args.status === 'live') return REPLAY_METADATA_POLL_MS
  if (args.hasData) return false
  if (args.endedAt == null) return false
  const now = args.now ?? Date.now()
  return now - args.endedAt < REPLAY_FINALIZE_GRACE_MS
    ? REPLAY_METADATA_POLL_MS
    : false
}

export interface UseReplayEventsVariables {
  sessionId: string
  /** Metadata revision used only to isolate client-side query snapshots. */
  revision?: string
}

/** Changes only when replay metadata says the downloadable event set changed. */
export function replayEventsRevision(
  metadata: RecordingMetadata | undefined,
): string | null {
  if (!metadata) return null
  return JSON.stringify([
    metadata.sizeBytes,
    metadata.lastEventAt ?? null,
    metadata.complete,
    metadata.tabs.map((tab) => [
      tab.tabId,
      tab.complete,
      tab.segments.map((segment) => [
        segment.documentId,
        segment.lastEventAt,
        segment.eventCount,
        segment.hasGap,
      ]),
    ]),
  ])
}

export interface ReplayEventsBundle {
  events: ReplayEvent[]
  tabIds: number[]
  documentIds: string[]
}

function isReplayEvent(value: unknown): value is ReplayEvent {
  if (!value || typeof value !== 'object') return false
  const event = value as Partial<ReplayEvent>
  return (
    typeof event.sessionId === 'string' &&
    typeof event.documentId === 'string' &&
    (event.targetId === null || typeof event.targetId === 'string') &&
    typeof event.tabId === 'number' &&
    typeof event.ts === 'number' &&
    typeof event.type === 'number'
  )
}

/**
 * Fetches and parses one session's tab-attributed, document-keyed NDJSON stream.
 * Parsing stays here so malformed recorder lines remain isolated from the
 * transport client and a missing recording still maps to an empty bundle.
 */
export async function fetchReplayEvents({
  sessionId,
}: UseReplayEventsVariables): Promise<ReplayEventsBundle> {
  let ndjson: string
  try {
    ndjson = await (await apiClient()).downloadRecordingEvents({ sessionId })
  } catch (error) {
    if (error instanceof ApiResponseError && error.response.status === 404) {
      return { events: [], tabIds: [], documentIds: [] }
    }
    throw error
  }

  const events: ReplayEvent[] = []
  const tabIds: number[] = []
  const documentIds: string[] = []
  const seenTabs = new Set<number>()
  const seenDocuments = new Set<string>()
  for (const line of ndjson.split('\n')) {
    if (line.length === 0) continue
    try {
      const event: unknown = JSON.parse(line)
      if (!isReplayEvent(event)) continue
      events.push(event)
      if (!seenTabs.has(event.tabId)) {
        seenTabs.add(event.tabId)
        tabIds.push(event.tabId)
      }
      if (!seenDocuments.has(event.documentId)) {
        seenDocuments.add(event.documentId)
        documentIds.push(event.documentId)
      }
    } catch {}
  }
  return { events, tabIds, documentIds }
}

export const useReplayEvents = createQuery<
  ReplayEventsBundle,
  UseReplayEventsVariables
>({
  queryKey: ['replay', 'events'],
  fetcher: fetchReplayEvents,
  staleTime: Number.POSITIVE_INFINITY,
})
