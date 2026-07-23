/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import type { ReplayTabData } from './replay.data'
import { buildReplayEventCatalog } from './replay-events'
import {
  buildSessionReplayPlan,
  projectGlobalTimeToTrack,
} from './session-replay'

function frame(
  t: number,
  cameraT: number,
  tabId: number | null,
  dispatchId?: number,
): ReplayFrame {
  return {
    t,
    cameraT,
    kind: 'action',
    verb: 'read',
    node: `dispatch-${dispatchId ?? 'missing'}`,
    caption: 'test',
    tabId,
    dispatchId,
  }
}

function event(
  ts: number,
  documentId: string,
  tabId: number,
  type = 2,
): ReplayEvent {
  return {
    sessionId: 'session-1',
    documentId,
    targetId: `target-${documentId}`,
    tabId,
    type,
    data: {},
    ts,
  }
}

function tab(tabId: number, documentIds: string[]): ReplayTabData {
  return {
    tabId,
    complete: true,
    segments: documentIds.map((documentId) => ({
      documentId,
      targetId: `target-${documentId}`,
      firstEventAt: 0,
      lastEventAt: 0,
      hasGap: false,
      legacy: false,
    })),
  }
}

describe('buildSessionReplayPlan', () => {
  it('builds one stitched track per Chrome tab and projects from absolute session time', () => {
    const events = [
      event(11_000, 'document-a', 1, 4),
      event(11_001, 'document-a', 1),
      event(15_000, 'document-a', 1, 3),
      event(20_000, 'document-b', 1, 4),
      event(20_001, 'document-b', 1),
      event(25_000, 'document-b', 1, 3),
    ]
    const catalog = buildReplayEventCatalog(events)
    const plan = buildSessionReplayPlan({
      totalSeconds: 30,
      startedAtMs: 10_000,
      frames: [frame(2, 1, 1, 1), frame(12, 11, 1, 2)],
      tabs: [tab(1, ['document-a', 'document-b'])],
      eventsForTab: catalog.eventsForTab,
    })

    expect(plan.tracks).toHaveLength(1)
    const track = plan.tracks[0]
    if (!track) throw new Error('expected a playable tab track')
    expect(track.events.map(({ documentId }) => documentId)).toEqual([
      'document-a',
      'document-a',
      'document-a',
      'document-b',
      'document-b',
      'document-b',
    ])
    expect(track.globalStartSeconds).toBe(1)
    expect(track.globalEndSeconds).toBe(15)
    expect(projectGlobalTimeToTrack(track, 12)).toBe(11)
  })

  it('keeps no-visual actions in the plan but excludes their tab from camera candidates', () => {
    const events = [
      event(11_000, 'playable', 1, 4),
      event(11_001, 'playable', 1),
      event(20_000, 'playable', 1, 3),
      event(12_000, 'mutation-only', 2, 3),
    ]
    const catalog = buildReplayEventCatalog(events)
    const noVisualFrame = frame(3, 2, 2, 22)
    const plan = buildSessionReplayPlan({
      totalSeconds: 30,
      startedAtMs: 10_000,
      frames: [frame(2, 1, 1, 11), noVisualFrame, frame(4, 3, null, 33)],
      tabs: [tab(1, ['playable']), tab(2, ['mutation-only'])],
      eventsForTab: catalog.eventsForTab,
    })

    expect(plan.frames).toContain(noVisualFrame)
    expect(plan.tracksByTab.get(2)?.hasFullSnapshot).toBe(false)
    expect(plan.cameraCandidates.map(({ tabId }) => tabId)).toEqual([1])
  })

  it('orders overlapping candidates by camera time, completion, dispatch id, then source order', () => {
    const events = [
      event(12_000, 'tab-one', 1, 4),
      event(12_001, 'tab-one', 1),
      event(30_000, 'tab-one', 1, 3),
    ]
    const catalog = buildReplayEventCatalog(events)
    const plan = buildSessionReplayPlan({
      totalSeconds: 30,
      startedAtMs: 10_000,
      frames: [
        frame(8, 5, 1, 4),
        frame(7, 5, 1, 9),
        frame(7, 5, 1, 3),
        frame(7, 4, 1, 8),
        frame(7, 5, 1),
        frame(7, 5, 1),
      ],
      tabs: [tab(1, ['tab-one'])],
      eventsForTab: catalog.eventsForTab,
    })

    expect(
      plan.cameraCandidates.map(
        ({ cameraT, completionT, dispatchId, sourceIndex }) => ({
          cameraT,
          completionT,
          dispatchId,
          sourceIndex,
        }),
      ),
    ).toEqual([
      { cameraT: 4, completionT: 7, dispatchId: 8, sourceIndex: 3 },
      { cameraT: 5, completionT: 7, dispatchId: 3, sourceIndex: 2 },
      { cameraT: 5, completionT: 7, dispatchId: 9, sourceIndex: 1 },
      { cameraT: 5, completionT: 7, dispatchId: null, sourceIndex: 4 },
      { cameraT: 5, completionT: 7, dispatchId: null, sourceIndex: 5 },
      { cameraT: 5, completionT: 8, dispatchId: 4, sourceIndex: 0 },
    ])
  })

  it('does not schedule a candidate before its tab can render', () => {
    const events = [
      event(15_000, 'late-tab', 1, 4),
      event(15_001, 'late-tab', 1),
      event(20_000, 'late-tab', 1, 3),
    ]
    const catalog = buildReplayEventCatalog(events)
    const plan = buildSessionReplayPlan({
      totalSeconds: 20,
      startedAtMs: 10_000,
      frames: [frame(8, 1, 1, 1)],
      tabs: [tab(1, ['late-tab'])],
      eventsForTab: catalog.eventsForTab,
    })

    expect(plan.cameraCandidates[0]?.cameraT).toBe(5)
  })
})
