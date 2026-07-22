/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import type { ReplayTabData } from './replay.data'

export interface TabView {
  frames: ReplayFrame[]
  /** Every playable document lifecycle from one Chrome tab. */
  events: readonly ReplayEvent[]
  totalSeconds: number
  hasFullSnapshot: boolean
  knownIncomplete: boolean
  /** Captured time omitted before the first playable checkpoint. */
  incompleteUntilMs: number | null
}

export const EMPTY_TAB_VIEW: TabView = {
  frames: [],
  events: [],
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
  frames: ReplayFrame[]
  tabs: ReplayTabData[]
  eventsForTab: (tabId: number) => readonly ReplayEvent[]
  startedAtMs: number
}

/** Projects one continuous player and action clock for a persisted Chrome tab. */
export function buildTabView(
  input: BuildTabViewInput,
  tabId: number | null,
): TabView {
  if (tabId === null) return EMPTY_TAB_VIEW
  const tab = input.tabs.find((candidate) => candidate.tabId === tabId)
  if (!tab) return EMPTY_TAB_VIEW

  const rawEvents = input.eventsForTab(tabId)
  const stream = playableTabStream(rawEvents)
  const rawFrames = input.frames.filter((frame) => frame.tabId === tabId)
  if (rawFrames.length === 0 && rawEvents.length === 0) return EMPTY_TAB_VIEW

  const hasFullSnapshot = stream.events.some((event) => event.type === 2)
  const timingEvents = hasFullSnapshot ? stream.events : rawEvents
  const originMs =
    timingEvents[0]?.ts ?? input.startedAtMs + (rawFrames[0]?.t ?? 0) * 1000
  const endMs =
    timingEvents.at(-1)?.ts ??
    input.startedAtMs + (rawFrames.at(-1)?.t ?? 0) * 1000
  const originT = (originMs - input.startedAtMs) / 1000
  const hasCatalogedGap =
    tab.complete === false ||
    tab.segments.some((segment) => segment.hasGap || segment.legacy)
  return {
    frames: rawFrames.map((frame) => ({
      ...frame,
      t: Math.max(0, frame.t - originT),
    })),
    events: stream.events,
    totalSeconds: Math.max(0, (endMs - originMs) / 1000),
    hasFullSnapshot,
    knownIncomplete: hasCatalogedGap || stream.hasOmittedEvents,
    incompleteUntilMs:
      hasCatalogedGap || stream.hasOmittedEventsAfterPlaybackStart
        ? null
        : stream.incompleteUntilMs,
  }
}

export interface TabSeek {
  tabId: number | null
  seconds: number
}

/** Resolves an audit frame to its persisted Chrome tab and continuous clock. */
export function tabSeekForFrame(
  input: BuildTabViewInput,
  selectedTabId: number | null,
  frame: ReplayFrame,
): TabSeek {
  const tabId = frame.tabId ?? selectedTabId
  if (tabId === null) return { tabId, seconds: frame.t }

  const view = buildTabView(input, tabId)
  const originMs =
    view.events[0]?.ts ?? input.eventsForTab(tabId)[0]?.ts ?? input.startedAtMs
  const timestamp = input.startedAtMs + frame.t * 1000
  return {
    tabId,
    seconds: Math.max(0, (timestamp - originMs) / 1000),
  }
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
