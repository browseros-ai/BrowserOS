/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import type { ReplayFrame } from '@/modules/api/replay.hooks'
import {
  createSessionCameraState,
  rebaseSessionCameraState,
  sessionCameraReducer,
} from './session-camera'
import type {
  CameraCandidate,
  SessionReplayPlan,
  TabTrack,
} from './session-replay'

function frame(
  t: number,
  tabId: number | null,
  dispatchId: number,
): ReplayFrame {
  return {
    t,
    cameraT: t,
    kind: 'action',
    verb: 'read',
    node: `dispatch-${dispatchId}`,
    caption: `Dispatch ${dispatchId}`,
    tabId,
    dispatchId,
  }
}

function track(
  tabId: number,
  globalStartSeconds: number,
  globalEndSeconds: number,
  hasFullSnapshot = true,
): TabTrack {
  return {
    tabId,
    frames: [],
    events: [],
    globalStartSeconds,
    globalEndSeconds,
    totalSeconds: globalEndSeconds - globalStartSeconds,
    hasFullSnapshot,
    knownIncomplete: false,
    incompleteUntilMs: null,
  }
}

function candidate(
  cameraT: number,
  tabId: number,
  dispatchId: number,
): CameraCandidate {
  const candidateFrame = frame(cameraT, tabId, dispatchId)
  return {
    frame: candidateFrame,
    tabId,
    cameraT,
    completionT: cameraT,
    dispatchId,
    sourceIndex: dispatchId,
  }
}

function plan(
  tracks: TabTrack[],
  cameraCandidates: CameraCandidate[] = [],
): SessionReplayPlan {
  return {
    totalSeconds: Math.max(
      ...tracks.map(({ globalEndSeconds }) => globalEndSeconds),
      0,
    ),
    frames: cameraCandidates.map(({ frame: candidateFrame }) => candidateFrame),
    tracks,
    tracksByTab: new Map(tracks.map((tabTrack) => [tabTrack.tabId, tabTrack])),
    cameraCandidates,
    firstPlayableTabId:
      tracks.find(({ hasFullSnapshot }) => hasFullSnapshot)?.tabId ?? null,
  }
}

function tick(
  replayPlan: SessionReplayPlan,
  state: ReturnType<typeof createSessionCameraState>,
  globalSeconds: number,
  realDeltaMs: number,
  playing = true,
) {
  return sessionCameraReducer(replayPlan, state, {
    type: 'tick',
    globalSeconds,
    realDeltaMs,
    playing,
  })
}

describe('sessionCameraReducer', () => {
  it('starts follow mode on the first camera-eligible track', () => {
    const replayPlan = plan([
      track(1, 0, 10, false),
      track(2, 1, 20),
      track(3, 2, 20),
    ])

    expect(createSessionCameraState(replayPlan)).toMatchObject({
      mode: 'follow',
      activeTabId: 2,
      pendingTabId: null,
      globalSeconds: 0,
      dwellMs: 0,
      isPlaying: true,
    })
  })

  it.each([
    { speed: 1, globalSeconds: 10 },
    { speed: 2, globalSeconds: 20 },
    { speed: 4, globalSeconds: 40 },
  ])('promotes after ten real seconds at $speed x', ({ globalSeconds }) => {
    const replayPlan = plan(
      [track(1, 0, 100), track(2, 1, 100)],
      [candidate(1, 2, 2)],
    )
    const initial = createSessionCameraState(replayPlan)

    const promoted = tick(replayPlan, initial, globalSeconds, 10_000)

    expect(promoted.activeTabId).toBe(2)
    expect(promoted.dwellMs).toBe(0)
  })

  it('does not charge paused wall time to the dwell window', () => {
    const replayPlan = plan(
      [track(1, 0, 100), track(2, 1, 100)],
      [candidate(1, 2, 2)],
    )
    const initial = createSessionCameraState(replayPlan)

    const paused = tick(replayPlan, initial, 1, 50_000, false)
    const resumed = tick(replayPlan, paused, 2, 9_999, true)

    expect(paused.dwellMs).toBe(0)
    expect(resumed.activeTabId).toBe(1)
    expect(resumed.pendingTabId).toBe(2)
  })

  it('coalesces parallel candidates and displays only the most recent tab', () => {
    const replayPlan = plan(
      [track(1, 0, 100), track(2, 1, 100), track(3, 2, 100)],
      [candidate(1, 2, 2), candidate(2, 3, 3)],
    )
    const initial = createSessionCameraState(replayPlan)

    const dwelling = tick(replayPlan, initial, 3, 5_000)
    const promoted = tick(replayPlan, dwelling, 4, 5_000)

    expect(dwelling.activeTabId).toBe(1)
    expect(dwelling.pendingTabId).toBe(3)
    expect(promoted.activeTabId).toBe(3)
  })

  it('promotes early when the active track has no playable time left', () => {
    const replayPlan = plan(
      [track(1, 0, 6), track(2, 1, 100)],
      [candidate(1, 2, 2)],
    )
    const initial = createSessionCameraState(replayPlan)

    const promoted = tick(replayPlan, initial, 6, 1_000)

    expect(promoted.activeTabId).toBe(2)
  })

  it('re-resolves an expired newest candidate to the latest playable candidate in the dwell window', () => {
    const replayPlan = plan(
      [track(1, 0, 100), track(2, 1, 20), track(3, 2, 8)],
      [candidate(1, 2, 2), candidate(2, 3, 3)],
    )
    const initial = createSessionCameraState(replayPlan)

    const newestPending = tick(replayPlan, initial, 6, 5_000)
    const promoted = tick(replayPlan, newestPending, 10, 5_000)

    expect(newestPending.pendingTabId).toBe(3)
    expect(promoted.activeTabId).toBe(2)
  })

  it('ignores candidates with missing tracks or unplayable visuals', () => {
    const replayPlan = plan(
      [track(1, 0, 100), track(2, 0, 100, false)],
      [candidate(1, 99, 99), candidate(2, 2, 2)],
    )
    const initial = createSessionCameraState(replayPlan)

    const afterCandidates = tick(replayPlan, initial, 3, 20_000)

    expect(afterCandidates.activeTabId).toBe(1)
    expect(afterCandidates.pendingTabId).toBeNull()
  })

  it('holds an exhausted final visual, then immediately promotes the next eligible candidate', () => {
    const replayPlan = plan(
      [track(1, 0, 5), track(2, 8, 30)],
      [candidate(8, 2, 2)],
    )
    const initial = createSessionCameraState(replayPlan)

    const held = tick(replayPlan, initial, 6, 1_000)
    const promoted = tick(replayPlan, held, 8, 500)

    expect(held.activeTabId).toBe(1)
    expect(promoted.activeTabId).toBe(2)
  })

  it('seeks backward by resetting the candidate cursor and resolving the latest playable camera', () => {
    const replayPlan = plan(
      [track(1, 0, 100), track(2, 10, 30), track(3, 20, 100)],
      [candidate(10, 2, 2), candidate(20, 3, 3)],
    )
    const initial = createSessionCameraState(replayPlan)
    const late = tick(replayPlan, initial, 25, 10_000)

    const sought = sessionCameraReducer(replayPlan, late, {
      type: 'seek',
      globalSeconds: 15,
    })

    expect(sought).toMatchObject({
      activeTabId: 2,
      pendingTabId: null,
      candidateCursor: 1,
      candidateWindowStartCursor: 1,
      dwellMs: 0,
      isPlaying: false,
      globalSeconds: 15,
    })
  })

  it("prefers a selected timeline frame's playable tab at completion time", () => {
    const replayPlan = plan(
      [track(1, 0, 100), track(2, 10, 30), track(3, 10, 30)],
      [candidate(10, 2, 2), candidate(11, 3, 3)],
    )
    const initial = createSessionCameraState(replayPlan)

    const selected = sessionCameraReducer(replayPlan, initial, {
      type: 'select-frame',
      frame: frame(12, 2, 20),
    })

    expect(selected.activeTabId).toBe(2)
    expect(selected.globalSeconds).toBe(12)
    expect(selected.isPlaying).toBe(false)
  })

  it('preserves the global position through inspect mode and resumes follow mode paused', () => {
    const replayPlan = plan(
      [track(1, 0, 100), track(2, 10, 100)],
      [candidate(10, 2, 2)],
    )
    const initial = createSessionCameraState(replayPlan)
    const atTwelve = tick(replayPlan, initial, 12, 4_000)

    const inspecting = sessionCameraReducer(replayPlan, atTwelve, {
      type: 'inspect',
      tabId: 1,
      globalSeconds: 12,
    })
    const resumed = sessionCameraReducer(replayPlan, inspecting, {
      type: 'resume',
    })

    expect(inspecting).toMatchObject({
      mode: 'inspect',
      activeTabId: 1,
      inspectTabId: 1,
      inspectSeconds: 0,
      resumeGlobalSeconds: 12,
      isPlaying: false,
      pendingTabId: null,
    })
    expect(resumed).toMatchObject({
      mode: 'follow',
      activeTabId: 2,
      globalSeconds: 12,
      resumeGlobalSeconds: null,
      isPlaying: false,
    })
  })

  it('restarts the clock, cursor, dwell, pending state, and initial camera', () => {
    const replayPlan = plan(
      [track(1, 0, 100), track(2, 10, 100)],
      [candidate(10, 2, 2)],
    )
    const initial = createSessionCameraState(replayPlan)
    const advanced = tick(replayPlan, initial, 20, 10_000)

    const restarted = sessionCameraReducer(replayPlan, advanced, {
      type: 'restart',
    })

    expect(restarted).toEqual(initial)
  })

  it('rebases a live candidate inserted before the prior cursor without replaying old work', () => {
    const replayPlan = plan(
      [track(1, 0, 100), track(2, 0, 100), track(3, 0, 100)],
      [candidate(1, 2, 2)],
    )
    const promoted = tick(
      replayPlan,
      createSessionCameraState(replayPlan),
      10,
      10_000,
    )
    expect(promoted).toMatchObject({
      activeTabId: 2,
      candidateCursor: 1,
      candidateWindowStartCursor: 1,
      pendingTabId: null,
    })

    const refreshedPlan = plan(
      [...replayPlan.tracks],
      [candidate(0.5, 3, 3), candidate(1, 2, 2)],
    )
    const rebased = rebaseSessionCameraState(
      replayPlan,
      refreshedPlan,
      promoted,
    )
    expect(rebased).toMatchObject({
      activeTabId: 2,
      candidateCursor: 2,
      candidateWindowStartCursor: 2,
      pendingTabId: 3,
    })

    expect(tick(refreshedPlan, rebased, 11, 10_000).activeTabId).toBe(3)
  })
})
