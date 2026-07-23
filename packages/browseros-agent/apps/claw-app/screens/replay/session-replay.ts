/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Immutable session-replay model shared by the global clock, camera reducer,
 * and viewport orchestration. Globally ordered dispatches schedule where the
 * camera looks; each TabTrack keeps one Chrome tab's rrweb DOM stream isolated.
 */

import type { ReplayFrame } from '@/modules/api/replay.hooks'
import {
  type BuildTabViewInput,
  buildTabView,
  projectGlobalTimeToTab,
  type TabView,
} from './tab-view'

export interface TabTrack extends TabView {
  tabId: number
}

export interface CameraCandidate {
  /** Frame remains the completion-timestamped action shown in the timeline. */
  frame: ReplayFrame
  tabId: number
  /**
   * Effective camera time. An operation may start before rrweb can draw its
   * tab, so eligibility is delayed to the first playable event.
   */
  cameraT: number
  completionT: number
  dispatchId: number | null
  /** Final deterministic tie-breaker when persisted identities are absent. */
  sourceIndex: number
}

export interface SessionReplayPlan {
  totalSeconds: number
  /** All actions remain available even when their tab has no drawable stream. */
  frames: readonly ReplayFrame[]
  tracks: readonly TabTrack[]
  tracksByTab: ReadonlyMap<number, TabTrack>
  cameraCandidates: readonly CameraCandidate[]
  firstPlayableTabId: number | null
}

export interface BuildSessionReplayPlanInput extends BuildTabViewInput {
  totalSeconds: number
}

/**
 * Builds the semantic schedule once per replay snapshot. Candidate sorting is
 * intentionally independent of input order until every persisted key is
 * exhausted; source order is only the last fallback for legacy rows.
 */
export function buildSessionReplayPlan(
  input: BuildSessionReplayPlanInput,
): SessionReplayPlan {
  const tracks = input.tabs.map(
    ({ tabId }): TabTrack => ({
      tabId,
      ...buildTabView(input, tabId),
    }),
  )
  const tracksByTab = new Map(tracks.map((track) => [track.tabId, track]))
  const cameraCandidates = input.frames
    .map((frame, sourceIndex): CameraCandidate | null => {
      if (frame.tabId === null || frame.tabId === undefined) return null
      const track = tracksByTab.get(frame.tabId)
      if (!track?.hasFullSnapshot) return null
      return {
        frame,
        tabId: frame.tabId,
        cameraT: Math.max(frame.cameraT, track.globalStartSeconds),
        completionT: frame.t,
        dispatchId: frame.dispatchId ?? null,
        sourceIndex,
      }
    })
    .filter((candidate): candidate is CameraCandidate => candidate !== null)
    .sort(compareCameraCandidates)

  return {
    totalSeconds: input.totalSeconds,
    frames: input.frames,
    tracks,
    tracksByTab,
    cameraCandidates,
    firstPlayableTabId:
      tracks.find((track) => track.hasFullSnapshot)?.tabId ?? null,
  }
}

function compareCameraCandidates(
  left: CameraCandidate,
  right: CameraCandidate,
): number {
  return (
    left.cameraT - right.cameraT ||
    left.completionT - right.completionT ||
    compareDispatchIds(left.dispatchId, right.dispatchId) ||
    left.sourceIndex - right.sourceIndex
  )
}

function compareDispatchIds(left: number | null, right: number | null): number {
  if (left === null && right === null) return 0
  if (left === null) return 1
  if (right === null) return -1
  return left - right
}

/** Converts global session time to the local offset consumed by one rrweb tab. */
export function projectGlobalTimeToTrack(
  track: TabTrack,
  globalSeconds: number,
): number {
  return projectGlobalTimeToTab(track, globalSeconds)
}

/**
 * Camera eligibility is stricter than local-time projection: projection holds
 * boundary visuals, while a tab may take over only when the global playhead is
 * inside its real recorded interval.
 */
export function isTrackPlayableAt(
  track: TabTrack,
  globalSeconds: number,
): boolean {
  return (
    track.hasFullSnapshot &&
    globalSeconds >= track.globalStartSeconds &&
    globalSeconds <= track.globalEndSeconds
  )
}
