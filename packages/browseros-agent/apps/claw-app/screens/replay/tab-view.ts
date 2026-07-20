/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import type { ReplaySegmentData, ReplayTabData } from './replay.data'

export interface TabView {
  frames: ReplayFrame[]
  /** Events from exactly one Chrome document lifecycle. */
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
const playableEvents = new WeakMap<
  readonly ReplayEvent[],
  readonly ReplayEvent[]
>()

export interface BuildTabViewInput {
  frames: ReplayFrame[]
  tabs: ReplayTabData[]
  eventsForDocument: (documentId: string) => readonly ReplayEvent[]
  startedAtMs: number
}

/** Builds one navigation segment without merging independent rrweb documents. */
export function buildTabView(
  input: BuildTabViewInput,
  tabId: number | null,
  documentId: string | null,
): TabView {
  const segment = findSegment(input.tabs, tabId, documentId)
  if (!segment || tabId === null) return EMPTY_TAB_VIEW
  const rawEvents = input.eventsForDocument(segment.documentId)
  const rawFrames = input.frames.filter((frame) => {
    if (frame.tabId !== tabId) return false
    const timestamp = input.startedAtMs + frame.t * 1000
    const tabSegments =
      input.tabs.find((candidate) => candidate.tabId === tabId)?.segments ?? []
    return (
      segmentForTimestamp(tabSegments, timestamp)?.documentId === documentId
    )
  })
  if (rawFrames.length === 0 && rawEvents.length === 0) return EMPTY_TAB_VIEW

  const firstSnapshotIndex = rawEvents.findIndex((event) => event.type === 2)
  const hasFullSnapshot = firstSnapshotIndex !== -1
  const missingSnapshot = rawEvents.length > 0 && !hasFullSnapshot
  const hasLeadingMutation =
    firstSnapshotIndex > 0 &&
    rawEvents.slice(0, firstSnapshotIndex).some((event) => event.type === 3)
  let events: readonly ReplayEvent[]
  if (!hasFullSnapshot) {
    events = NO_VISUAL_EVENTS
  } else if (hasLeadingMutation) {
    const cached = playableEvents.get(rawEvents)
    events = cached ?? rawEvents.slice(firstSnapshotIndex)
    if (!cached) playableEvents.set(rawEvents, events)
  } else {
    events = rawEvents
  }
  const incompleteUntilMs = hasLeadingMutation
    ? Math.max(
        0,
        (rawEvents[firstSnapshotIndex]?.ts ?? 0) - (rawEvents[0]?.ts ?? 0),
      )
    : null
  const timingEvents = hasFullSnapshot ? events : rawEvents
  const originMs =
    timingEvents[0]?.ts ?? input.startedAtMs + (rawFrames[0]?.t ?? 0) * 1000
  const endMs =
    timingEvents.at(-1)?.ts ??
    input.startedAtMs + (rawFrames.at(-1)?.t ?? 0) * 1000
  const originT = (originMs - input.startedAtMs) / 1000
  return {
    frames: rawFrames.map((frame) => ({
      ...frame,
      t: Math.max(0, frame.t - originT),
    })),
    events,
    totalSeconds: Math.max(0, (endMs - originMs) / 1000),
    hasFullSnapshot,
    knownIncomplete:
      segment.hasGap || segment.legacy || missingSnapshot || hasLeadingMutation,
    incompleteUntilMs,
  }
}

export interface TabSeek {
  tabId: number | null
  documentId: string | null
  seconds: number
}

/** Resolves an audit frame to its logical tab and navigation segment clock. */
export function tabSeekForFrame(
  input: BuildTabViewInput,
  selectedTabId: number | null,
  selectedDocumentId: string | null,
  frame: ReplayFrame,
): TabSeek {
  const tabId = frame.tabId ?? selectedTabId
  if (tabId === null) {
    return { tabId, documentId: selectedDocumentId, seconds: frame.t }
  }
  const tab = input.tabs.find((candidate) => candidate.tabId === tabId)
  const timestamp = input.startedAtMs + frame.t * 1000
  const segment =
    segmentForTimestamp(tab?.segments ?? [], timestamp) ??
    findSegment(input.tabs, tabId, selectedDocumentId)
  if (!segment) return { tabId, documentId: null, seconds: frame.t }
  const view = buildTabView(input, tabId, segment.documentId)
  const originMs =
    view.events[0]?.ts ?? segment.firstEventAt ?? input.startedAtMs
  return {
    tabId,
    documentId: segment.documentId,
    seconds: Math.max(0, (timestamp - originMs) / 1000),
  }
}

function findSegment(
  tabs: readonly ReplayTabData[],
  tabId: number | null,
  documentId: string | null,
): ReplaySegmentData | undefined {
  if (tabId === null || documentId === null) return undefined
  return tabs
    .find((tab) => tab.tabId === tabId)
    ?.segments.find((segment) => segment.documentId === documentId)
}

function segmentForTimestamp(
  segments: readonly ReplaySegmentData[],
  timestamp: number,
): ReplaySegmentData | undefined {
  const overlapping = segments.find(
    (segment) =>
      timestamp >= segment.firstEventAt && timestamp <= segment.lastEventAt,
  )
  if (overlapping) return overlapping
  return [...segments].sort((left, right) => {
    const leftDistance = Math.min(
      Math.abs(timestamp - left.firstEventAt),
      Math.abs(timestamp - left.lastEventAt),
    )
    const rightDistance = Math.min(
      Math.abs(timestamp - right.firstEventAt),
      Math.abs(timestamp - right.lastEventAt),
    )
    return leftDistance - rightDistance
  })[0]
}
