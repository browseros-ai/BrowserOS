/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ReplayEvent } from '@/modules/api/replay.hooks'
import type { ReplayTabData } from './replay.data'

export interface TabView {
  /** Every playable document lifecycle from one Chrome tab. */
  events: readonly ReplayEvent[]
  /**
   * Absolute epoch ms of the first and last playable event. Null when the
   * tab recorded nothing renderable. `session-replay.ts` anchors the global
   * activity window on these, so they stay absolute rather than rebased.
   */
  originMs: number | null
  endMs: number | null
  /** Length of this tab's own rrweb stream. */
  totalSeconds: number
  hasFullSnapshot: boolean
  knownIncomplete: boolean
  /** Captured time omitted before the first playable checkpoint. */
  incompleteUntilMs: number | null
}

export const EMPTY_TAB_VIEW: TabView = {
  events: [],
  originMs: null,
  endMs: null,
  totalSeconds: 0,
  hasFullSnapshot: false,
  knownIncomplete: false,
  incompleteUntilMs: null,
}

const NO_VISUAL_EVENTS: readonly ReplayEvent[] = []

interface PlayableTabStream {
  events: readonly ReplayEvent[]
  hasOmittedEvents: boolean
  hasOmittedEventsAfterPlaybackStart: boolean
  incompleteUntilMs: number | null
}

const playableTabStreams = new WeakMap<
  readonly ReplayEvent[],
  PlayableTabStream
>()

const EMPTY_PLAYABLE_TAB_STREAM: PlayableTabStream = {
  events: NO_VISUAL_EVENTS,
  hasOmittedEvents: false,
  hasOmittedEventsAfterPlaybackStart: false,
  incompleteUntilMs: null,
}

interface DocumentState {
  firstSnapshotIndex: number
  mutationBeforeSnapshot: boolean
}

export interface BuildTabViewInput {
  tabs: readonly ReplayTabData[]
  eventsForTab: (tabId: number) => readonly ReplayEvent[]
}

/** Projects one continuous rrweb track for a persisted Chrome tab. */
export function buildTabView(
  input: BuildTabViewInput,
  tabId: number | null,
): TabView {
  if (tabId === null) return EMPTY_TAB_VIEW
  const tab = input.tabs.find((candidate) => candidate.tabId === tabId)
  if (!tab) return EMPTY_TAB_VIEW

  const rawEvents = input.eventsForTab(tabId)
  const stream = playableTabStream(rawEvents)
  const hasFullSnapshot = stream.events.some((event) => event.type === 2)
  // Without a snapshot nothing is renderable, so the raw stream still
  // describes when the tab was alive for the incomplete-recording notice.
  const timingEvents = hasFullSnapshot ? stream.events : rawEvents
  const originMs = timingEvents[0]?.ts ?? null
  const endMs = timingEvents.at(-1)?.ts ?? null
  const hasCatalogedGap =
    tab.complete === false ||
    tab.segments.some((segment) => segment.hasGap || segment.legacy)
  return {
    events: stream.events,
    originMs,
    endMs,
    totalSeconds:
      originMs === null || endMs === null
        ? 0
        : Math.max(0, (endMs - originMs) / 1000),
    hasFullSnapshot,
    knownIncomplete: hasCatalogedGap || stream.hasOmittedEvents,
    incompleteUntilMs:
      hasCatalogedGap || stream.hasOmittedEventsAfterPlaybackStart
        ? null
        : stream.incompleteUntilMs,
  }
}

/** True when rrweb can reconstruct this tab: a snapshot plus something after it. */
export function isPlayableTabView(view: TabView): boolean {
  return view.hasFullSnapshot && view.events.length >= 2
}

function playableTabStream(
  rawEvents: readonly ReplayEvent[],
): PlayableTabStream {
  if (rawEvents.length === 0) return EMPTY_PLAYABLE_TAB_STREAM
  const cached = playableTabStreams.get(rawEvents)
  if (cached) return cached

  const documents = new Map<string, DocumentState>()
  rawEvents.forEach((event, index) => {
    const state = documents.get(event.documentId) ?? {
      firstSnapshotIndex: -1,
      mutationBeforeSnapshot: false,
    }
    if (state.firstSnapshotIndex === -1) {
      if (event.type === 3) state.mutationBeforeSnapshot = true
      if (event.type === 2) state.firstSnapshotIndex = index
    }
    documents.set(event.documentId, state)
  })

  const hasOmittedEvents = [...documents.values()].some(
    (state) => state.firstSnapshotIndex === -1 || state.mutationBeforeSnapshot,
  )
  let playbackStarted = false
  let hasOmittedEventsAfterPlaybackStart = false
  const events = hasOmittedEvents
    ? rawEvents.filter((event, index) => {
        const state = documents.get(event.documentId)
        const playable =
          state !== undefined &&
          state.firstSnapshotIndex !== -1 &&
          (!state.mutationBeforeSnapshot || index >= state.firstSnapshotIndex)
        if (playable) playbackStarted = true
        else if (playbackStarted) hasOmittedEventsAfterPlaybackStart = true
        return playable
      })
    : rawEvents
  const firstRawAt = rawEvents[0]?.ts
  const firstPlayableAt = events[0]?.ts
  const result = {
    events,
    hasOmittedEvents,
    hasOmittedEventsAfterPlaybackStart,
    incompleteUntilMs:
      firstRawAt !== undefined &&
      firstPlayableAt !== undefined &&
      firstPlayableAt > firstRawAt
        ? firstPlayableAt - firstRawAt
        : null,
  }
  playableTabStreams.set(rawEvents, result)
  return result
}
