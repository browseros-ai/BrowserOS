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
  type SessionReplayPlanInput,
} from './session-replay'

const SESSION_START_MS = 1_000_000

function frame(
  t: number,
  tabId: number | null,
  extra: Partial<ReplayFrame> = {},
): ReplayFrame {
  return {
    t,
    kind: 'action',
    verb: 'read',
    node: 'test',
    caption: `action at ${t}`,
    tabId,
    ...extra,
  }
}

function event(
  ts: number,
  documentId: string,
  tabId = 1,
  type = 2,
): ReplayEvent {
  return {
    sessionId: 'test',
    documentId,
    targetId: `target-${documentId}`,
    tabId,
    type,
    data: {},
    ts,
  }
}

/** Snapshot plus a trailing mutation: the minimum rrweb can reconstruct. */
function track(tabId: number, originMs: number, endMs: number): ReplayEvent[] {
  const documentId = `document-${tabId}`
  return [
    event(originMs, documentId, tabId, 2),
    event(endMs, documentId, tabId, 3),
  ]
}

function tabs(...tabIds: number[]): ReplayTabData[] {
  return tabIds.map((tabId) => ({ tabId, complete: true, segments: [] }))
}

function makeInput(
  overrides: Partial<SessionReplayPlanInput> = {},
): SessionReplayPlanInput {
  return {
    frames: [],
    tabs: [],
    eventsForTab: () => [],
    startedAtMs: SESSION_START_MS,
    ...overrides,
  }
}

function planFor(
  events: readonly ReplayEvent[],
  frames: readonly ReplayFrame[],
  tabIds: number[],
) {
  return buildSessionReplayPlan(
    makeInput({
      frames,
      tabs: tabs(...tabIds),
      eventsForTab: buildReplayEventCatalog(events).eventsForTab,
    }),
  )
}

describe('activity bounds', () => {
  it('starts at the first playable checkpoint, not session start', () => {
    const plan = planFor(
      track(1, SESSION_START_MS + 4_000, SESSION_START_MS + 9_000),
      [frame(6, 1)],
      [1],
    )

    expect(plan.tracks).toHaveLength(1)
    expect(plan.tracks[0]?.startAt).toBe(0)
    expect(plan.firstPlayableTabId).toBe(1)
    // The 6s dispatch sits 2s after the checkpoint at 4s.
    expect(plan.actions[0]?.completionAt).toBe(2)
  })

  it('ends at the later of the last tool and the last rrweb event', () => {
    const visualsEndLater = planFor(
      track(1, SESSION_START_MS, SESSION_START_MS + 20_000),
      [frame(5, 1)],
      [1],
    )
    expect(visualsEndLater.durationSeconds).toBe(20)

    const toolsEndLater = planFor(
      track(1, SESSION_START_MS, SESSION_START_MS + 9_000),
      [frame(38, 1)],
      [1],
    )
    expect(toolsEndLater.durationSeconds).toBe(38)
  })

  it('reports no replayable activity when nothing is playable', () => {
    const plan = planFor(
      [event(SESSION_START_MS, 'document-1', 1, 3)],
      [frame(4, 1), frame(9, null)],
      [1],
    )

    expect(plan.durationSeconds).toBe(0)
    expect(plan.tracks).toEqual([])
    expect(plan.firstPlayableTabId).toBeNull()
    // The global timeline still lists everything, timed from session start.
    expect(plan.actions.map(({ startAt }) => startAt)).toEqual([4, 9])
    // The camera still names a tab so the page can show its no-recording state.
    expect(plan.stateAt(0).tabId).toBe(1)
    expect(plan.trackFor(1)).toBeNull()
  })
})

describe('action schedule', () => {
  it('derives start from a valid duration and falls back to completion', () => {
    const plan = planFor(
      track(1, SESSION_START_MS, SESSION_START_MS + 40_000),
      [
        frame(10, 1, { durationMs: 4_000, dispatchId: 1 }),
        frame(20, 1, { durationMs: undefined, dispatchId: 2 }),
        frame(30, 1, { durationMs: -5_000, dispatchId: 3 }),
        frame(35, 1, { durationMs: Number.NaN, dispatchId: 4 }),
        frame(38, 1, { durationMs: Number.POSITIVE_INFINITY, dispatchId: 5 }),
      ],
      [1],
    )

    expect(plan.actions.map(({ startAt }) => startAt)).toEqual([
      6, 20, 30, 35, 38,
    ])
  })

  it('clamps a start that precedes the activity origin', () => {
    const plan = planFor(
      track(1, SESSION_START_MS + 5_000, SESSION_START_MS + 20_000),
      [frame(6, 1, { durationMs: 6_000 })],
      [1],
    )

    // Completion is 1s into activity; a 6s tool would have begun 5s earlier.
    expect(plan.actions[0]?.startAt).toBe(0)
    expect(plan.actions[0]?.completionAt).toBe(1)
  })

  it('orders overlapping starts by completion, dispatch id, then source order', () => {
    const plan = planFor(
      track(1, SESSION_START_MS, SESSION_START_MS + 30_000),
      [
        // Every tool overlaps at start 10; only the tie-breaks separate them.
        frame(12, 1, { durationMs: 2_000, dispatchId: 40, caption: 'latest' }),
        frame(11, 1, { durationMs: 1_000, dispatchId: 30, caption: 'middle' }),
        frame(10, 1, { durationMs: 0, dispatchId: 20, caption: 'higher-id' }),
        frame(10, 1, { durationMs: 0, dispatchId: 10, caption: 'lower-id' }),
        frame(10, 1, { durationMs: 0, caption: 'no-id-first' }),
        frame(10, 1, { durationMs: 0, caption: 'no-id-second' }),
      ],
      [1],
    )

    expect(plan.actions.map(({ startAt }) => startAt)).toEqual([
      10, 10, 10, 10, 10, 10,
    ])
    expect(plan.actions.map(({ frame: f }) => f.caption)).toEqual([
      'lower-id',
      'higher-id',
      'no-id-first',
      'no-id-second',
      'middle',
      'latest',
    ])
  })

  it('does not mutate the source frame array', () => {
    const frames = [
      frame(20, 1, { dispatchId: 2 }),
      frame(10, 1, { dispatchId: 1 }),
    ]
    planFor(track(1, SESSION_START_MS, SESSION_START_MS + 30_000), frames, [1])

    expect(frames.map(({ dispatchId }) => dispatchId)).toEqual([2, 1])
  })
})

describe('stateAt', () => {
  const events = [
    ...track(1, SESSION_START_MS, SESSION_START_MS + 10_000),
    ...track(2, SESSION_START_MS + 20_000, SESSION_START_MS + 30_000),
  ]
  const frames = [
    frame(5, 1, { dispatchId: 1, caption: 'tab 1 tool' }),
    frame(15, null, { dispatchId: 2, caption: 'name_session' }),
    frame(25, 2, { dispatchId: 3, caption: 'tab 2 tool' }),
    frame(28, 3, { dispatchId: 4, caption: 'no-visual tab tool' }),
  ]
  const plan = planFor(events, frames, [1, 2, 3])

  it('shows the first playable track before any action starts', () => {
    expect(plan.stateAt(0).currentIndex).toBe(-1)
    expect(plan.stateAt(0).currentAction).toBeNull()
    expect(plan.stateAt(0).tabId).toBe(1)
    expect(plan.project(1, 0)).toEqual({ trackTime: 0, live: true })
  })

  it('follows the latest crossed action that has a playable tab', () => {
    expect(plan.stateAt(5).tabId).toBe(1)
    expect(plan.stateAt(25).tabId).toBe(2)
  })

  it('keeps an untabbed action current without moving the camera', () => {
    const state = plan.stateAt(15)
    expect(state.currentAction?.frame.caption).toBe('name_session')
    expect(state.currentAction?.trackTabId).toBeNull()
    expect(state.tabId).toBe(1)
  })

  it('keeps a no-visual tab action current without moving the camera', () => {
    const state = plan.stateAt(28)
    expect(state.currentAction?.frame.caption).toBe('no-visual tab tool')
    expect(state.currentAction?.trackTabId).toBeNull()
    expect(state.tabId).toBe(2)
  })

  it('resolves identically regardless of query order', () => {
    const forward = [0, 5, 15, 25, 28].map((t) => plan.stateAt(t))
    const backward = [28, 25, 15, 5, 0].map((t) => plan.stateAt(t)).reverse()

    expect(backward).toEqual(forward)
  })

  it('collapses several actions crossed in one update to the latest tab', () => {
    // Jumping 0 -> 28 crosses tab 1, the untabbed tool, tab 2, and a no-visual
    // tab; only the latest playable tab may win.
    expect(plan.stateAt(28).tabId).toBe(2)
    expect(plan.stateAt(28).currentIndex).toBe(3)
  })
})

describe('projection', () => {
  const plan = planFor(
    [
      ...track(1, SESSION_START_MS, SESSION_START_MS + 10_000),
      ...track(2, SESSION_START_MS + 20_000, SESSION_START_MS + 30_000),
    ],
    [frame(5, 1), frame(25, 2)],
    [1, 2],
  )

  it('holds the first checkpoint before a track begins recording', () => {
    expect(plan.project(2, 5)).toEqual({ trackTime: 0, live: false })
  })

  it('tracks global time inside the recorded range', () => {
    expect(plan.project(2, 25)).toEqual({ trackTime: 5, live: true })
  })

  it('holds the final recorded state after a track ends', () => {
    expect(plan.project(1, 25)).toEqual({ trackTime: 10, live: false })
    expect(plan.project(1, 10)).toEqual({ trackTime: 10, live: false })
  })

  it('returns a clamped projection for a tab with no track', () => {
    expect(plan.project(99, 5)).toEqual({ trackTime: 0, live: false })
    expect(plan.project(null, 5)).toEqual({ trackTime: 0, live: false })
  })

  it('never plays a static single-checkpoint track', () => {
    const staticPlan = planFor(
      [
        event(SESSION_START_MS, 'document-1', 1, 2),
        event(SESSION_START_MS, 'document-1', 1, 4),
      ],
      [frame(3, 1)],
      [1],
    )

    expect(staticPlan.tracks[0]?.durationSeconds).toBe(0)
    expect(staticPlan.project(1, 0).live).toBe(false)
    expect(staticPlan.durationSeconds).toBe(3)
  })
})

describe('address bar', () => {
  const plan = planFor(
    [
      ...track(1, SESSION_START_MS, SESSION_START_MS + 30_000),
      ...track(2, SESSION_START_MS + 5_000, SESSION_START_MS + 30_000),
    ],
    [
      frame(2, 1, { dispatchId: 1, url: 'https://one.example/a' }),
      frame(6, 2, { dispatchId: 2, url: 'https://two.example/b' }),
      frame(8, 1, { dispatchId: 3, url: null }),
      frame(10, null, { dispatchId: 4, url: 'https://untabbed.example' }),
    ],
    [1, 2],
  )

  it('keeps the visible tab URL when a later action carries none', () => {
    expect(plan.urlAt(1, 10)).toBe('https://one.example/a')
  })

  it('does not leak another tab URL into the visible tab', () => {
    expect(plan.urlAt(2, 10)).toBe('https://two.example/b')
  })

  it('has no URL before the tab is first seen', () => {
    expect(plan.urlAt(2, 2)).toBeNull()
    expect(plan.urlAt(null, 10)).toBeNull()
  })
})

describe('same-tab navigation', () => {
  it('stays one playable track across document lifecycles', () => {
    const plan = planFor(
      [
        event(SESSION_START_MS, 'document-a', 1, 2),
        event(SESSION_START_MS + 4_000, 'document-a', 1, 3),
        event(SESSION_START_MS + 6_000, 'document-b', 1, 2),
        event(SESSION_START_MS + 12_000, 'document-b', 1, 3),
      ],
      [frame(10, 1)],
      [1],
    )

    expect(plan.tracks).toHaveLength(1)
    expect(plan.tracks[0]?.durationSeconds).toBe(12)
    expect(plan.project(1, 10)).toEqual({ trackTime: 10, live: true })
  })
})

describe('Trendshift regression', () => {
  // Real 2026-07-24 session: tab 1 stopped emitting DOM events at 9.003s while
  // wait/read tools kept targeting it out to 38.321s. Chapter playback ended at
  // 9.003s, so the last three tools could never become current.
  const TAB_1_VISUAL_END = 9.003
  const TAB_1_TOOLS = [2.053, 24.225, 26.717, 38.321]
  const plan = planFor(
    [
      ...track(1, SESSION_START_MS, SESSION_START_MS + TAB_1_VISUAL_END * 1000),
      ...track(2, SESSION_START_MS + 12_000, SESSION_START_MS + 18_000),
    ],
    [
      ...TAB_1_TOOLS.map((t, index) =>
        frame(t, 1, { dispatchId: index + 1, caption: `tab 1 tool ${t}` }),
      ),
      frame(14, null, { dispatchId: 90, caption: 'name_session' }),
      frame(16, 2, { dispatchId: 91, caption: 'tab 2 tool' }),
    ],
    [1, 2],
  )

  it('keeps every late tool reachable inside the activity window', () => {
    expect(plan.durationSeconds).toBe(38.321)
    for (const t of TAB_1_TOOLS) {
      expect(plan.stateAt(t).currentAction?.frame.caption).toBe(
        `tab 1 tool ${t}`,
      )
    }
  })

  it('holds the final tab 1 frame while its late tools stay current', () => {
    for (const t of [24.225, 26.717, 38.321]) {
      expect(plan.stateAt(t).tabId).toBe(1)
      expect(plan.project(1, t)).toEqual({
        trackTime: TAB_1_VISUAL_END,
        live: false,
      })
    }
  })

  it('still switches to tab 2 and back to tab 1 in tool order', () => {
    expect(plan.stateAt(16).tabId).toBe(2)
    expect(plan.stateAt(24.225).tabId).toBe(1)
  })

  it('keeps the untabbed name_session current without a camera move', () => {
    const state = plan.stateAt(14)
    expect(state.currentAction?.frame.caption).toBe('name_session')
    expect(state.tabId).toBe(1)
  })
})
