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
import { createQuery } from 'react-query-kit'
import { apiClient, resolveApiBaseUrl } from './client'
import { parseResponse } from './parseResponse'

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
  /** Seconds into the session. */
  t: number
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

export const useReplayMetadata = createQuery<
  ReplayMetadata,
  UseReplayMetadataVariables
>({
  queryKey: ['replay', 'metadata'],
  fetcher: fetchReplayMetadata,
  refetchInterval: 10_000,
})

export interface UseReplayEventsVariables {
  sessionId: string
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
 * This streaming read stays raw so malformed lines can be skipped and a 404
 * can map to an empty bundle instead of the typed client's error path.
 */
export async function fetchReplayEvents({
  sessionId,
}: UseReplayEventsVariables): Promise<ReplayEventsBundle> {
  const baseUrl = await resolveApiBaseUrl()
  const res = await fetch(
    `${baseUrl}/api/v1/sessions/${encodeURIComponent(sessionId)}/recording/events`,
  )
  if (!res.ok) {
    if (res.status === 404) {
      return { events: [], tabIds: [], documentIds: [] }
    }
    return parseResponse<ReplayEventsBundle>(res)
  }

  const events: ReplayEvent[] = []
  const tabIds: number[] = []
  const documentIds: string[] = []
  const seenTabs = new Set<number>()
  const seenDocuments = new Set<string>()
  for (const line of (await res.text()).split('\n')) {
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
