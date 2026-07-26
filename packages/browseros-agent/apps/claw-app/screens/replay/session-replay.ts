/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Immutable projection of one session into a single chronological replay.
 *
 * The page owns global activity time; everything else is answered here as a
 * pure function of that time — which tool is current, which Chrome tab the
 * camera should show, and where that tab's bounded rrweb stream sits. Deriving
 * rather than accumulating is what lets a forward seek, a backward seek, and a
 * live plan swap all resolve to identical state with no camera history.
 *
 * The load-bearing case is a tab whose DOM recording stops long before its last
 * tool call: real sessions contain tabs with ~9s of rrweb events and valid
 * tab-attributed tools past 38s. Tools stay reachable on the global clock while
 * the visible track clamps to its final recorded frame.
 */

import type { ReplayEvent, ReplayFrame } from '@/modules/api/replay.hooks'
import type { ReplayTabData } from './replay.data'
import {
  type BuildTabViewInput,
  buildTabView,
  EMPTY_TAB_VIEW,
  isPlayableTabView,
  type TabView,
} from './tab-view'

export interface SessionReplayPlanInput extends BuildTabViewInput {
  frames: readonly ReplayFrame[]
  tabs: readonly ReplayTabData[]
  eventsForTab: (tabId: number) => readonly ReplayEvent[]
  /** Session start in epoch ms; the origin of every frame's `t`. */
  startedAtMs: number
}

export interface ReplayAction {
  /** Source dispatch row. `t` stays the session-relative completion offset. */
  frame: ReplayFrame
  /**
   * Activity seconds where this action becomes current. Completion minus a
   * valid duration, clamped at the activity origin.
   */
  startAt: number
  /**
   * Activity seconds of the source completion timestamp. Negative when the
   * tool finished before the first recording checkpoint; used only to break
   * ordering ties.
   */
  completionAt: number
  /** Tab to show for this action, or null when it must not move the camera. */
  trackTabId: number | null
  /** Position in the source dispatch list; final ordering tie-break. */
  sourceIndex: number
}

export interface ReplayTrack {
  tabId: number
  view: TabView
  /** Activity seconds where this track's recording begins. */
  startAt: number
  /** Length of the track's own rrweb stream. */
  durationSeconds: number
}

export interface ReplayState {
  /** Index into `actions`; -1 before the first action starts. */
  currentIndex: number
  currentAction: ReplayAction | null
  /** Tab the camera follows: latest crossed playable tab, else the first one. */
  tabId: number | null
}

export interface TrackProjection {
  /** Seconds into the track's own rrweb stream, clamped to its range. */
  trackTime: number
  /**
   * True while global time lies inside the recorded range. False means the
   * track is holding a boundary frame — its first checkpoint before the
   * recording started, its final state after it ended — so rrweb must stay
   * paused there even while global playback continues.
   */
  live: boolean
}

export interface SessionReplayPlan {
  /** Replayable activity; 0 when no tab recorded anything renderable. */
  durationSeconds: number
  /** Every tool in one global chronological schedule. */
  actions: readonly ReplayAction[]
  /** Playable tracks, earliest recording first. */
  tracks: readonly ReplayTrack[]
  /** Tab shown before any action with visuals has been crossed. */
  firstPlayableTabId: number | null
  /** Any known tab, playable or not, for warnings and the pinned-tab state. */
  viewFor: (tabId: number | null) => TabView
  trackFor: (tabId: number | null) => ReplayTrack | null
  stateAt: (time: number) => ReplayState
  project: (tabId: number | null, time: number) => TrackProjection
  /** Latest URL this tab was known to be on at `time`. */
  urlAt: (tabId: number | null, time: number) => string | null
}

const NO_ACTIONS: readonly ReplayAction[] = []
const NO_TRACKS: readonly ReplayTrack[] = []
const CLAMPED_AT_START: TrackProjection = { trackTime: 0, live: false }
const NO_STATE: ReplayState = {
  currentIndex: -1,
  currentAction: null,
  tabId: null,
}

export const EMPTY_SESSION_REPLAY_PLAN: SessionReplayPlan = {
  durationSeconds: 0,
  actions: NO_ACTIONS,
  tracks: NO_TRACKS,
  firstPlayableTabId: null,
  viewFor: () => EMPTY_TAB_VIEW,
  trackFor: () => null,
  stateAt: () => NO_STATE,
  project: () => CLAMPED_AT_START,
  urlAt: () => null,
}

/** Builds the immutable replay plan for one accepted revision of session data. */
export function buildSessionReplayPlan(
  input: SessionReplayPlanInput,
): SessionReplayPlan {
  const views = new Map<number, TabView>(
    input.tabs.map((tab) => [tab.tabId, buildTabView(input, tab.tabId)]),
  )
  const playable = [...views.entries()].flatMap(([tabId, view]) =>
    isPlayableTabView(view) && view.originMs !== null && view.endMs !== null
      ? [{ tabId, view, originMs: view.originMs, endMs: view.endMs }]
      : [],
  )
  playable.sort(
    (left, right) => left.originMs - right.originMs || left.tabId - right.tabId,
  )

  // Without a checkpoint there is nothing to replay, so activity has no length.
  // Actions are still scheduled against session start to keep the global
  // timeline readable, and the page renders its no-recording state instead of a
  // transport. Anchoring on the first checkpoint (rather than session start)
  // also keeps the origin immutable as a live recording grows.
  const first = playable[0]
  const originMs = first?.originMs ?? input.startedAtMs
  const actions = buildActions(
    input,
    originMs,
    new Set(playable.map(({ tabId }) => tabId)),
  )
  const lastVisualEnd = playable.reduce(
    (latest, track) => Math.max(latest, (track.endMs - originMs) / 1000),
    0,
  )
  const lastCompletion = actions.reduce(
    (latest, action) => Math.max(latest, action.completionAt),
    0,
  )
  const durationSeconds = first ? Math.max(lastVisualEnd, lastCompletion) : 0

  const tracks: ReplayTrack[] = playable.map(
    ({ tabId, view, originMs: at }) => ({
      tabId,
      view,
      startAt: (at - originMs) / 1000,
      durationSeconds: view.totalSeconds,
    }),
  )
  const tracksByTab = new Map(tracks.map((track) => [track.tabId, track]))
  // Latest playable tab among actions[0..i], so a lookup never rescans history.
  const cameraTabByIndex: (number | null)[] = []
  let carried: number | null = null
  for (const action of actions) {
    carried = action.trackTabId ?? carried
    cameraTabByIndex.push(carried)
  }

  const firstPlayableTabId = first?.tabId ?? null
  // With nothing playable anywhere, still name a tab so the page keeps a chip
  // selected and shows that tab's no-recording state rather than no tab at all.
  const cameraFallbackTabId = firstPlayableTabId ?? input.tabs[0]?.tabId ?? null
  const trackFor = (tabId: number | null): ReplayTrack | null =>
    tabId === null ? null : (tracksByTab.get(tabId) ?? null)
  const stateAt = (time: number): ReplayState => {
    const currentIndex = lastActionStartedAt(actions, time)
    return {
      currentIndex,
      currentAction: actions[currentIndex] ?? null,
      tabId:
        (currentIndex < 0 ? null : cameraTabByIndex[currentIndex]) ??
        cameraFallbackTabId,
    }
  }

  return {
    durationSeconds,
    actions,
    tracks,
    firstPlayableTabId,
    viewFor: (tabId) =>
      tabId === null ? EMPTY_TAB_VIEW : (views.get(tabId) ?? EMPTY_TAB_VIEW),
    trackFor,
    stateAt,
    project: (tabId, time) => {
      const track = trackFor(tabId)
      if (!track) return CLAMPED_AT_START
      const elapsed = time - track.startAt
      return {
        trackTime: Math.min(Math.max(0, elapsed), track.durationSeconds),
        live: elapsed >= 0 && elapsed < track.durationSeconds,
      }
    },
    urlAt: (tabId, time) => {
      if (tabId === null) return null
      for (let i = lastActionStartedAt(actions, time); i >= 0; i--) {
        const { frame } = actions[i] as ReplayAction
        if (frame.tabId === tabId && frame.url) return frame.url
      }
      return null
    },
  }
}

function buildActions(
  input: SessionReplayPlanInput,
  originMs: number,
  playableTabIds: ReadonlySet<number>,
): readonly ReplayAction[] {
  if (input.frames.length === 0) return NO_ACTIONS
  return input.frames
    .map((frame, sourceIndex): ReplayAction => {
      const completionAt =
        (input.startedAtMs + frame.t * 1000 - originMs) / 1000
      const duration = frame.durationMs
      const measured =
        typeof duration === 'number' &&
        Number.isFinite(duration) &&
        duration >= 0
          ? duration / 1000
          : 0
      return {
        frame,
        startAt: Math.max(0, completionAt - measured),
        completionAt,
        trackTabId:
          frame.tabId !== null &&
          frame.tabId !== undefined &&
          playableTabIds.has(frame.tabId)
            ? frame.tabId
            : null,
        sourceIndex,
      }
    })
    .sort(
      (left, right) =>
        left.startAt - right.startAt ||
        left.completionAt - right.completionAt ||
        dispatchOrder(left) - dispatchOrder(right) ||
        left.sourceIndex - right.sourceIndex,
    )
}

function dispatchOrder(action: ReplayAction): number {
  return action.frame.dispatchId ?? Number.MAX_SAFE_INTEGER
}

/** Index of the last action whose start has been crossed; -1 before the first. */
function lastActionStartedAt(
  actions: readonly ReplayAction[],
  time: number,
): number {
  let low = 0
  let high = actions.length - 1
  let found = -1
  while (low <= high) {
    const mid = (low + high) >> 1
    if ((actions[mid] as ReplayAction).startAt <= time) {
      found = mid
      low = mid + 1
    } else {
      high = mid - 1
    }
  }
  return found
}
