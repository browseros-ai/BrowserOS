/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  RecordingSegmentMetadata,
  RecordingTabMetadata,
} from '@browseros/claw-api'
import type { ReplayEvent } from '@/modules/api/replay.hooks'

export const EMPTY_REPLAY_EVENTS: readonly ReplayEvent[] = []

export interface ReplayEventCatalog {
  tabIds: number[]
  documentIdsForTab: (tabId: number) => readonly string[]
  eventsForDocument: (documentId: string) => readonly ReplayEvent[]
}

/** Keeps each Chrome document in its own stable rrweb event array. */
export function buildReplayEventCatalog(
  events: readonly ReplayEvent[],
): ReplayEventCatalog {
  if (events.length === 0) {
    return {
      tabIds: [],
      documentIdsForTab: () => [],
      eventsForDocument: () => EMPTY_REPLAY_EVENTS,
    }
  }

  const tabIds: number[] = []
  const seenTabs = new Set<number>()
  const documentsByTab = new Map<number, string[]>()
  const eventsByDocument = new Map<string, ReplayEvent[]>()
  for (const event of events) {
    if (!seenTabs.has(event.tabId)) {
      seenTabs.add(event.tabId)
      tabIds.push(event.tabId)
    }
    const documentEvents = eventsByDocument.get(event.documentId)
    if (documentEvents) {
      documentEvents.push(event)
      continue
    }
    eventsByDocument.set(event.documentId, [event])
    const documentIds = documentsByTab.get(event.tabId) ?? []
    documentIds.push(event.documentId)
    documentsByTab.set(event.tabId, documentIds)
  }

  return {
    tabIds,
    documentIdsForTab: (tabId) => documentsByTab.get(tabId) ?? [],
    eventsForDocument: (documentId) =>
      eventsByDocument.get(documentId) ?? EMPTY_REPLAY_EVENTS,
  }
}

/** Returns metadata-ordered logical tabs, falling back to stream discovery. */
export function buildReplayTabIds(
  tabs: readonly RecordingTabMetadata[] | undefined,
  discoveredTabIds: readonly number[],
): number[] {
  if (!tabs) return [...discoveredTabIds]
  const ordered = [...tabs]
    .sort((left, right) => left.firstEventAt - right.firstEventAt)
    .map((tab) => tab.tabId)
  return [
    ...ordered,
    ...discoveredTabIds.filter((tabId) => !ordered.includes(tabId)),
  ]
}

/** Returns metadata-ordered navigation segments for one logical tab. */
export function buildReplayDocumentIds(
  segments: readonly RecordingSegmentMetadata[] | undefined,
  discoveredDocumentIds: readonly string[],
): string[] {
  if (!segments) return [...discoveredDocumentIds]
  const ordered = [...segments]
    .sort((left, right) => left.firstEventAt - right.firstEventAt)
    .map((segment) => segment.documentId)
  return [
    ...ordered,
    ...discoveredDocumentIds.filter(
      (documentId) => !ordered.includes(documentId),
    ),
  ]
}
