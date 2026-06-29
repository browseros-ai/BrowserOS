/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Session-replay API surface for the claw-app cockpit.
 *
 * Two consumers, two hooks:
 *
 *   - `useReplayMetadata({ sessionId })` polls
 *     `GET /audit/replay/:sessionId/exists` (cheap) so the audit
 *     task page can flip its "View Session Replay" CTA between
 *     enabled and "no replay yet". Refetches while the page is
 *     open so a live session unlocks the CTA without a hard
 *     refresh.
 *
 *   - `useReplayEvents({ sessionId })` fetches the full NDJSON
 *     stream from `GET /audit/replay/:sessionId` and parses each
 *     line into an rrweb event. Mounted only by the replay page;
 *     the cache is keyed on sessionId so swapping between two
 *     audit sessions does not re-fetch when both are still in
 *     view.
 *
 * Type definitions for the visual frame timeline that the existing
 * `screens/replay/` scaffold consumes live here too. Frames are
 * derived in `screens/replay/replay.data.ts` from real
 * tool_dispatches, not from this file.
 */

import { createQuery } from 'react-query-kit'
import { api } from './client'
import { parseResponse } from './parseResponse'

export type ReplayVerb =
  | 'navigate'
  | 'read'
  | 'click'
  | 'type'
  | 'attach'
  | 'submit'
  | 'done'

export type ReplayKind = 'action' | 'approval' | 'block' | 'done'

export interface ReplayFrame {
  /** Seconds into the session. */
  t: number
  kind: ReplayKind
  verb: ReplayVerb
  /** Short node label, e.g. the page title or a focused element. */
  node: string
  /** Caption sentence rendered in the viewport overlay + timeline row. */
  caption: string
  /** Optional badge shown on the timeline row ("Blocked", "Cancelled"). */
  note?: string
  /** Source dispatch id so the replay surface can deep-link. */
  dispatchId?: number
}

/**
 * One rrweb event as parsed from the NDJSON stream. The on-disk line
 * carries `sessionId` (server-trusted) + `tabPageId` (recorder-supplied)
 * + the standard rrweb `{type, data, ts}`. The replay UI filters by
 * `tabPageId` to drive a single rrweb-player instance at a time.
 */
export interface ReplayEvent {
  sessionId: string
  tabPageId: number
  /** rrweb event type 0-5. */
  type: number
  data: unknown
  /** Capture timestamp, ms since epoch. */
  ts: number
}

export interface ReplayMetadata {
  ok: boolean
  hasData: boolean
  sizeBytes: number
  firstEventAt?: number
  lastEventAt?: number
  /** Distinct page ids that contributed events to this session. */
  tabPageIds: number[]
}

interface UseReplayMetadataVariables {
  sessionId: string
}

export const useReplayMetadata = createQuery<
  ReplayMetadata,
  UseReplayMetadataVariables
>({
  queryKey: ['replay', 'metadata'],
  fetcher: async ({ sessionId }) => {
    const res = await api.audit.replay[':sessionId'].exists.$get({
      param: { sessionId },
    })
    return parseResponse<ReplayMetadata>(res)
  },
  // While a live session is still streaming events the metadata
  // (sizeBytes, lastEventAt, tabPageIds) keeps changing. 10s is a
  // cheap poll over loopback and is what flips the CTA from
  // disabled to enabled the first time data arrives.
  refetchInterval: 10_000,
})

interface UseReplayEventsVariables {
  sessionId: string
}

export interface ReplayEventsBundle {
  events: ReplayEvent[]
  /** All distinct tabPageIds in the stream, sorted ascending. */
  tabPageIds: number[]
}

export const useReplayEvents = createQuery<
  ReplayEventsBundle,
  UseReplayEventsVariables
>({
  queryKey: ['replay', 'events'],
  fetcher: async ({ sessionId }) => {
    const res = await api.audit.replay[':sessionId'].$get({
      param: { sessionId },
    })
    if (!res.ok) {
      // 404 means no replay data; surface a clean empty bundle so
      // the UI can render its no-data state without an error boundary
      // catching the parseResponse throw.
      if (res.status === 404) return { events: [], tabPageIds: [] }
      return parseResponse<ReplayEventsBundle>(res)
    }
    const text = await res.text()
    const events: ReplayEvent[] = []
    const tabs = new Set<number>()
    for (const line of text.split('\n')) {
      if (line.length === 0) continue
      try {
        const ev = JSON.parse(line) as ReplayEvent
        if (
          typeof ev.ts === 'number' &&
          typeof ev.type === 'number' &&
          typeof ev.tabPageId === 'number'
        ) {
          events.push(ev)
          tabs.add(ev.tabPageId)
        }
      } catch {
        // Malformed line; the recorder shouldn't emit these, but if
        // a partial line ever sneaks in we skip it rather than abort
        // the whole stream.
      }
    }
    return {
      events,
      tabPageIds: [...tabs].sort((a, b) => a - b),
    }
  },
  // Replay events are immutable once a session ends; for live
  // sessions a manual refresh button is enough. No refetch interval.
  staleTime: Number.POSITIVE_INFINITY,
})
