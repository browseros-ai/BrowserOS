/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { type ReplayData, useReplayData } from './replay.data'
import {
  buildSessionReplayPlan,
  EMPTY_SESSION_REPLAY_PLAN,
  type ReplayAction,
  type ReplayState,
  type SessionReplayPlan,
} from './session-replay'
import { type Playback, usePlayback } from './use-playback'

export interface ReplaySession {
  replay: ReplayData | null
  isLoading: boolean
  plan: SessionReplayPlan
  /** Playback whose Play also resumes automatic tab following. */
  playback: Playback
  state: ReplayState
  /** The tab the camera shows: the pinned one, else the current action's. */
  visibleTabId: number | null
  pinnedTabId: number | null
  selectTab: (value: string) => void
  resumeFollow: () => void
  /** Seeks to an action and goes back to following the agent's tabs. */
  selectAction: (action: ReplayAction) => void
  seek: (time: number) => void
}

/**
 * One global activity clock for a session's replay. The session plan answers
 * which tool is current and which tab to show; nothing here accumulates camera
 * state, so seeking either way and live refreshes land on the same answer.
 * Pinning a tab pins only the camera, never the clock.
 */
export function useReplaySession(sessionId: string): ReplaySession {
  const { replay, isLoading } = useReplayData(sessionId)
  const [pinnedTabId, setPinnedTabId] = useState<number | null>(null)
  const activeSessionRef = useRef<string | null>(null)

  const plan = useMemo(
    () => (replay ? buildSessionReplayPlan(replay) : EMPTY_SESSION_REPLAY_PLAN),
    [replay],
  )
  const playback = usePlayback(plan.durationSeconds)
  const { pause, play, seek, togglePlay: togglePlayback } = playback

  useEffect(() => {
    const replaySessionId = replay?.sessionId
    if (
      replaySessionId === undefined ||
      activeSessionRef.current === replaySessionId
    ) {
      return
    }
    activeSessionRef.current = replaySessionId
    setPinnedTabId(null)
    seek(0)
    play()
  }, [replay?.sessionId, play, seek])

  const selectTab = useCallback(
    (value: string) => {
      const tabId = Number(value)
      if (!Number.isSafeInteger(tabId)) return
      pause()
      setPinnedTabId(tabId)
    },
    [pause],
  )

  const resumeFollow = useCallback(() => setPinnedTabId(null), [])

  // Play always resumes automatic following; pause leaves a pin in place so
  // the person can stop, inspect a tab, and step around without losing it.
  const togglePlay = useCallback(() => {
    if (!playback.isPlaying) setPinnedTabId(null)
    togglePlayback()
  }, [playback.isPlaying, togglePlayback])

  const selectAction = useCallback(
    (action: ReplayAction) => {
      setPinnedTabId(null)
      seek(action.startAt)
    },
    [seek],
  )

  const state = plan.stateAt(playback.time)

  return {
    replay,
    isLoading,
    plan,
    playback: { ...playback, togglePlay },
    state,
    visibleTabId: pinnedTabId ?? state.tabId,
    pinnedTabId,
    selectTab,
    resumeFollow,
    selectAction,
    seek,
  }
}
