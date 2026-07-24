/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure camera choreography for session replay. This state machine never talks
 * to React or rrweb: BrowserClaw supplies global session time plus real wall
 * time, and the reducer decides which independent tab film should be visible.
 */

import type { ReplayFrame } from '@/modules/api/replay.hooks'
import {
  type CameraCandidate,
  isTrackPlayableAt,
  type SessionReplayPlan,
} from './session-replay'

export const CAMERA_DWELL_MS = 10_000
const TIME_EPSILON_SECONDS = 0.001

export type SessionCameraMode = 'follow' | 'inspect'

export interface SessionCameraState {
  mode: SessionCameraMode
  activeTabId: number | null
  pendingTabId: number | null
  globalSeconds: number
  isPlaying: boolean
  /**
   * Number of sorted candidates already crossed by the global playhead. This
   * is a cursor, not an event id: candidates before it need no reconsideration
   * during ordinary forward playback.
   */
  candidateCursor: number
  /**
   * Inclusive beginning of the candidates observed during the active tab's
   * dwell. We retain this bounded logical window so an expired newest pending
   * tab can fall back to the latest earlier candidate still drawable now.
   */
  candidateWindowStartCursor: number
  /** Real playing time spent viewing the active tab; speed never scales it. */
  dwellMs: number
  inspectTabId: number | null
  inspectSeconds: number
  /** Exact app-clock position to restore after independent tab inspection. */
  resumeGlobalSeconds: number | null
}

export type SessionCameraAction =
  | {
      type: 'tick'
      globalSeconds: number
      realDeltaMs: number
      playing: boolean
    }
  | { type: 'seek'; globalSeconds: number }
  | { type: 'select-frame'; frame: ReplayFrame }
  | { type: 'inspect'; tabId: number; globalSeconds: number }
  | { type: 'resume' }
  | { type: 'restart' }

/** Initializes automatic following without consuming a visual-only tab. */
export function createSessionCameraState(
  plan: SessionReplayPlan,
): SessionCameraState {
  const candidateCursor = candidateCursorAt(plan, 0)
  return {
    mode: 'follow',
    activeTabId: plan.firstPlayableTabId,
    pendingTabId: null,
    globalSeconds: 0,
    isPlaying: true,
    candidateCursor,
    candidateWindowStartCursor: candidateCursor,
    dwellMs: 0,
    inspectTabId: null,
    inspectSeconds: 0,
    resumeGlobalSeconds: null,
  }
}

export function sessionCameraReducer(
  plan: SessionReplayPlan,
  state: SessionCameraState,
  action: SessionCameraAction,
): SessionCameraState {
  switch (action.type) {
    case 'tick':
      return tickCamera(plan, state, action)
    case 'seek':
      return resolveFollowAt(plan, action.globalSeconds)
    case 'select-frame':
      return resolveFollowAt(plan, action.frame.t, action.frame.tabId)
    case 'inspect':
      return enterInspection(plan, state, action.tabId, action.globalSeconds)
    case 'resume':
      return resumeFollowing(plan, state)
    case 'restart':
      return createSessionCameraState(plan)
  }
}

/**
 * Carries camera state across a live replay-plan refresh. Cursors are indexes
 * into an immutable snapshot, so reusing them against a newly sorted array can
 * skip a long-running dispatch whose completion arrived late but whose
 * completion-minus-duration start sorts before the old cursor.
 *
 * Existing dwell history is translated through stable candidate identities.
 * A genuinely new, already-crossed candidate is retained as pending even when
 * its semantic start belongs before that translated window; subsequent ticks
 * preserve it until it expires, is superseded, or is promoted.
 */
export function rebaseSessionCameraState(
  previousPlan: SessionReplayPlan,
  plan: SessionReplayPlan,
  state: SessionCameraState,
): SessionCameraState {
  const activeTrack =
    state.activeTabId === null
      ? undefined
      : plan.tracksByTab.get(state.activeTabId)
  const activeStillValid =
    state.mode === 'inspect'
      ? activeTrack !== undefined
      : activeTrack?.hasFullSnapshot === true
  if (!activeStillValid) {
    const resolved = resolveFollowAt(plan, state.globalSeconds)
    return { ...resolved, isPlaying: state.isPlaying }
  }

  const candidateCursor = candidateCursorAt(plan, state.globalSeconds)
  const candidateWindowStartCursor = Math.min(
    candidateCursor,
    translateWindowStart(previousPlan, plan, state.candidateWindowStartCursor),
  )
  let pendingTabId = resolvePendingFromWindow(
    plan,
    state.activeTabId,
    candidateWindowStartCursor,
    candidateCursor,
    state.globalSeconds,
  )

  if (
    pendingTabId === null &&
    state.pendingTabId !== null &&
    state.pendingTabId !== state.activeTabId
  ) {
    const priorPendingTrack = plan.tracksByTab.get(state.pendingTabId)
    if (
      priorPendingTrack &&
      isTrackPlayableAt(priorPendingTrack, state.globalSeconds)
    ) {
      pendingTabId = state.pendingTabId
    }
  }

  if (pendingTabId === null) {
    const previousCandidates = new Set(
      previousPlan.cameraCandidates.map(candidateIdentity),
    )
    for (let index = candidateCursor - 1; index >= 0; index -= 1) {
      const candidate = plan.cameraCandidates[index]
      if (
        !candidate ||
        previousCandidates.has(candidateIdentity(candidate)) ||
        candidate.tabId === state.activeTabId
      ) {
        continue
      }
      const track = plan.tracksByTab.get(candidate.tabId)
      if (track && isTrackPlayableAt(track, state.globalSeconds)) {
        pendingTabId = candidate.tabId
        break
      }
    }
  }

  return {
    ...state,
    candidateCursor,
    candidateWindowStartCursor,
    pendingTabId,
  }
}

function tickCamera(
  plan: SessionReplayPlan,
  state: SessionCameraState,
  action: Extract<SessionCameraAction, { type: 'tick' }>,
): SessionCameraState {
  if (state.mode === 'inspect') {
    return { ...state, isPlaying: false }
  }

  const globalSeconds = clampGlobal(plan, action.globalSeconds)
  if (globalSeconds + TIME_EPSILON_SECONDS < state.globalSeconds) {
    const resolved = resolveFollowAt(plan, globalSeconds)
    return { ...resolved, isPlaying: action.playing }
  }

  const candidateCursor = advanceCandidateCursor(
    plan,
    state.candidateCursor,
    globalSeconds,
  )
  const dwellMs = action.playing
    ? Math.min(
        CAMERA_DWELL_MS,
        state.dwellMs + usableRealDelta(action.realDeltaMs),
      )
    : state.dwellMs
  const windowPendingTabId = resolvePendingFromWindow(
    plan,
    state.activeTabId,
    state.candidateWindowStartCursor,
    candidateCursor,
    globalSeconds,
  )
  const retainedPendingTrack =
    state.pendingTabId === null
      ? undefined
      : plan.tracksByTab.get(state.pendingTabId)
  const pendingTabId =
    windowPendingTabId ??
    (state.pendingTabId !== state.activeTabId &&
    retainedPendingTrack &&
    isTrackPlayableAt(retainedPendingTrack, globalSeconds)
      ? state.pendingTabId
      : null)
  const activeTrack =
    state.activeTabId === null
      ? undefined
      : plan.tracksByTab.get(state.activeTabId)
  const activeTrackEnded =
    activeTrack === undefined ||
    globalSeconds >= activeTrack.globalEndSeconds - TIME_EPSILON_SECONDS
  const shouldPromote =
    action.playing &&
    pendingTabId !== null &&
    (dwellMs >= CAMERA_DWELL_MS || activeTrackEnded)

  if (shouldPromote) {
    return {
      ...state,
      activeTabId: pendingTabId,
      pendingTabId: null,
      globalSeconds,
      isPlaying: true,
      candidateCursor,
      candidateWindowStartCursor: candidateCursor,
      dwellMs: 0,
    }
  }

  return {
    ...state,
    pendingTabId,
    globalSeconds,
    isPlaying: action.playing,
    candidateCursor,
    dwellMs,
  }
}

function resolveFollowAt(
  plan: SessionReplayPlan,
  requestedSeconds: number,
  preferredTabId?: number | null,
): SessionCameraState {
  const globalSeconds = clampGlobal(plan, requestedSeconds)
  const candidateCursor = candidateCursorAt(plan, globalSeconds)
  const preferredTrack =
    preferredTabId === null || preferredTabId === undefined
      ? undefined
      : plan.tracksByTab.get(preferredTabId)
  let activeTabId =
    preferredTrack && isTrackPlayableAt(preferredTrack, globalSeconds)
      ? preferredTrack.tabId
      : null

  if (activeTabId === null) {
    for (let index = candidateCursor - 1; index >= 0; index -= 1) {
      const candidate = plan.cameraCandidates[index]
      if (!candidate) continue
      const track = plan.tracksByTab.get(candidate.tabId)
      if (track && isTrackPlayableAt(track, globalSeconds)) {
        activeTabId = candidate.tabId
        break
      }
    }
  }
  activeTabId ??= plan.firstPlayableTabId

  return {
    mode: 'follow',
    activeTabId,
    pendingTabId: null,
    globalSeconds,
    isPlaying: false,
    candidateCursor,
    candidateWindowStartCursor: candidateCursor,
    dwellMs: 0,
    inspectTabId: null,
    inspectSeconds: 0,
    resumeGlobalSeconds: null,
  }
}

function enterInspection(
  plan: SessionReplayPlan,
  state: SessionCameraState,
  tabId: number,
  requestedGlobalSeconds: number,
): SessionCameraState {
  const globalSeconds = clampGlobal(plan, requestedGlobalSeconds)
  return {
    ...state,
    mode: 'inspect',
    activeTabId: tabId,
    pendingTabId: null,
    globalSeconds,
    isPlaying: false,
    dwellMs: 0,
    inspectTabId: tabId,
    inspectSeconds: 0,
    resumeGlobalSeconds: globalSeconds,
  }
}

function resumeFollowing(
  plan: SessionReplayPlan,
  state: SessionCameraState,
): SessionCameraState {
  const restoredSeconds = state.resumeGlobalSeconds ?? state.globalSeconds
  // `resolveFollowAt` intentionally returns paused. Resume changes ownership
  // back to the global transport but never surprises the inspector by moving.
  return resolveFollowAt(plan, restoredSeconds)
}

/**
 * Scans newest-to-oldest only inside the current dwell window. Parallel work
 * therefore coalesces to one pending camera, while retaining enough history to
 * recover if that newest tab's short recording ends before promotion.
 */
function resolvePendingFromWindow(
  plan: SessionReplayPlan,
  activeTabId: number | null,
  windowStartCursor: number,
  candidateCursor: number,
  globalSeconds: number,
): number | null {
  for (
    let index = candidateCursor - 1;
    index >= windowStartCursor;
    index -= 1
  ) {
    const candidate = plan.cameraCandidates[index]
    if (!candidate || candidate.tabId === activeTabId) continue
    const track = plan.tracksByTab.get(candidate.tabId)
    if (track && isTrackPlayableAt(track, globalSeconds)) {
      return candidate.tabId
    }
  }
  return null
}

function candidateCursorAt(
  plan: SessionReplayPlan,
  globalSeconds: number,
): number {
  return advanceCandidateCursor(plan, 0, globalSeconds)
}

function advanceCandidateCursor(
  plan: SessionReplayPlan,
  initialCursor: number,
  globalSeconds: number,
): number {
  let cursor = initialCursor
  while (
    cursor < plan.cameraCandidates.length &&
    (plan.cameraCandidates[cursor]?.cameraT ?? Number.POSITIVE_INFINITY) <=
      globalSeconds + TIME_EPSILON_SECONDS
  ) {
    cursor += 1
  }
  return cursor
}

function clampGlobal(plan: SessionReplayPlan, seconds: number): number {
  if (!Number.isFinite(seconds)) return 0
  return Math.max(0, Math.min(plan.totalSeconds, seconds))
}

function usableRealDelta(milliseconds: number): number {
  return Number.isFinite(milliseconds) && milliseconds > 0 ? milliseconds : 0
}

function translateWindowStart(
  previousPlan: SessionReplayPlan,
  plan: SessionReplayPlan,
  previousWindowStart: number,
): number {
  for (
    let index = Math.min(
      previousWindowStart - 1,
      previousPlan.cameraCandidates.length - 1,
    );
    index >= 0;
    index -= 1
  ) {
    const boundary = previousPlan.cameraCandidates[index]
    if (!boundary) continue
    const identity = candidateIdentity(boundary)
    const translatedIndex = plan.cameraCandidates.findIndex(
      (candidate) => candidateIdentity(candidate) === identity,
    )
    if (translatedIndex !== -1) return translatedIndex + 1
  }
  return 0
}

function candidateIdentity(candidate: CameraCandidate): string {
  return candidate.dispatchId !== null
    ? `dispatch:${candidate.dispatchId}`
    : [
        'legacy',
        candidate.tabId,
        candidate.cameraT,
        candidate.completionT,
        candidate.sourceIndex,
      ].join(':')
}
