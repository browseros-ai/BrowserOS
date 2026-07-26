/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { ArrowLeft, History } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useLocation } from 'react-router'
import { StatusBadge } from '@/components/cockpit/StatusBadge'
import { Spinner } from '@/components/ui/spinner'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { EventTimeline } from './EventTimeline'
import { PlaybackTransport } from './PlaybackTransport'
import { ReplayViewport } from './ReplayViewport'
import { useReplayData } from './replay.data'
import {
  buildSessionReplayPlan,
  EMPTY_SESSION_REPLAY_PLAN,
  type ReplayAction,
} from './session-replay'
import { usePlayback } from './use-playback'

/**
 * Renders the audit replay page.
 *
 * The page owns one global activity clock and asks the session plan which tool
 * is current and which tab to show. Nothing here accumulates camera state, so a
 * backward seek, a forward seek, and a live data refresh all land on the same
 * answer. The operator can pin a tab to inspect it; that pins the camera only —
 * the clock and the action schedule never fork.
 */
export function Replay() {
  const { replay, isLoading, navigate } = useReplayData()
  const location = useLocation()
  const [pinnedTabId, setPinnedTabId] = useState<number | null>(null)
  const activeSessionRef = useRef<string | null>(null)

  const plan = useMemo(
    () => (replay ? buildSessionReplayPlan(replay) : EMPTY_SESSION_REPLAY_PLAN),
    [replay],
  )
  const playback = usePlayback(plan.durationSeconds)
  const { pause, play, seek, togglePlay: togglePlayback } = playback

  useEffect(() => {
    const sessionId = replay?.sessionId
    if (sessionId === undefined || activeSessionRef.current === sessionId) {
      return
    }
    activeSessionRef.current = sessionId
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

  // Play always resumes automatic following; pause leaves a pin in place so an
  // operator can stop, inspect a tab, and step around without losing it.
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

  const transportPlayback = { ...playback, togglePlay }
  const state = plan.stateAt(playback.time)
  const visibleTabId = pinnedTabId ?? state.tabId
  const track = plan.trackFor(visibleTabId)
  const view = plan.viewFor(visibleTabId)
  const projection = plan.project(visibleTabId, playback.time)

  if (isLoading || !replay) {
    return (
      <div className="flex h-full flex-1 items-center justify-center bg-bg-canvas text-ink-3">
        <Spinner />
      </div>
    )
  }

  // navigate(-1) preserves task detail's original location.state.from
  // (the entry we're moving back to is re-focused, not re-created), so
  // task detail's Back button keeps its cockpit / audit-list target.
  // Doing navigate(`/audit/${sessionId}`) instead would push a new
  // history entry and lose that state.
  //
  // Signal for "reached replay via the in-app flow": task detail's
  // View Replay button seeds location.state.from with the referring
  // pathname. Absence of that flag means direct URL / refresh, so we
  // fall back to the semantic parent. window.history.length is not
  // used because it counts the whole tab's browser history, not just
  // SPA-internal navigations, and can misfire on any prior entry.
  const cameFromInAppFlow =
    typeof location.state === 'object' &&
    location.state !== null &&
    'from' in location.state &&
    typeof (location.state as { from: unknown }).from === 'string'
  const back = () =>
    cameFromInAppFlow ? navigate(-1) : navigate(`/audit/${replay.sessionId}`)
  const stats: { label: string; value: string }[] = [
    { label: 'Duration', value: replay.duration },
    { label: 'Steps', value: replay.steps },
  ]

  return (
    <div className="flex h-screen min-h-0 flex-col bg-bg-canvas">
      <header className="flex shrink-0 items-center gap-4 border-border border-b bg-card px-5 py-3">
        <button
          type="button"
          onClick={back}
          className="flex items-center gap-1.5 font-semibold text-ink-2 text-sm hover:text-ink"
        >
          <ArrowLeft className="size-4" />
          Audit trail
        </button>
        <span className="h-5 w-px bg-border-2" />
        <span className="inline-flex items-center gap-1.5 rounded-full bg-accent-tint px-2.5 py-0.5 font-bold text-[10.5px] text-accent-ink uppercase tracking-wider">
          <History className="size-3" />
          Replay
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate font-bold text-ink text-sm">
            {replay.taskTitle}
          </div>
          <div className="text-ink-3 text-xs">
            {replay.agentLabel} · {replay.harness}
            {replay.startedAt ? ` · ${replay.startedAt}` : ''}
          </div>
        </div>
        <StatusBadge status={replay.status} />
        <div className="ml-2 flex gap-5">
          {stats.map((stat) => (
            <div key={stat.label}>
              <div className="font-bold text-[10px] text-ink-4 uppercase tracking-wider">
                {stat.label}
              </div>
              <div className="font-bold font-mono text-ink text-sm">
                {stat.value}
              </div>
            </div>
          ))}
        </div>
      </header>

      <div className="flex min-h-0 flex-1">
        <div className="flex min-w-0 flex-1 flex-col gap-3 p-4">
          {replay.tabs.length > 1 && (
            <div className="flex min-h-9 items-center gap-3">
              <Tabs
                value={visibleTabId === null ? '' : visibleTabId.toString()}
                onValueChange={selectTab}
              >
                <TabsList variant="line">
                  {replay.tabs.map(({ tabId }, idx) => (
                    <TabsTrigger
                      key={tabId}
                      value={tabId.toString()}
                      onClick={
                        tabId === visibleTabId
                          ? () => selectTab(tabId.toString())
                          : undefined
                      }
                    >
                      Tab {idx + 1}
                    </TabsTrigger>
                  ))}
                </TabsList>
              </Tabs>
              {pinnedTabId !== null && (
                <button
                  type="button"
                  data-resume-follow
                  onClick={resumeFollow}
                  className="ml-auto shrink-0 rounded-full bg-accent-tint px-2.5 py-1 font-semibold text-accent-ink text-xs"
                >
                  Resume follow
                </button>
              )}
            </div>
          )}
          {view.incompleteUntilMs !== null && (
            <div
              role="status"
              className="rounded-lg border border-amber/30 bg-amber-tint px-3 py-2 font-medium text-ink-2 text-xs"
            >
              Recording incomplete — playback starts at{' '}
              {formatIncompleteOffset(view.incompleteUntilMs)}
            </div>
          )}
          {view.incompleteUntilMs === null && view.knownIncomplete && (
            <div
              role="status"
              className="rounded-lg border border-amber/30 bg-amber-tint px-3 py-2 font-medium text-ink-2 text-xs"
            >
              Recording incomplete — this replay contains a known gap
            </div>
          )}
          <div className="relative flex min-h-0 flex-1 flex-col gap-3">
            <div className="flex min-h-0 flex-1">
              {track ? (
                <ReplayViewport
                  site={replay.site}
                  action={state.currentAction?.frame}
                  url={plan.urlAt(visibleTabId, playback.time)}
                  events={track.view.events}
                  trackTime={projection.trackTime}
                  live={projection.live}
                  isPlaying={playback.isPlaying}
                  speed={playback.speed}
                />
              ) : (
                <div className="flex flex-1 items-center justify-center rounded-2xl border border-border-2 bg-card text-ink-3 text-sm shadow-sm">
                  No visual recording for this tab
                </div>
              )}
            </div>
            {plan.durationSeconds > 0 && (
              <PlaybackTransport
                playback={transportPlayback}
                totalSeconds={plan.durationSeconds}
                actions={plan.actions}
                onSeek={seek}
              />
            )}
          </div>
        </div>
        <EventTimeline
          actions={plan.actions}
          currentIndex={state.currentIndex}
          onSelectAction={selectAction}
        />
      </div>
    </div>
  )
}

function formatIncompleteOffset(milliseconds: number): string {
  const totalSeconds = Math.floor(milliseconds / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}:${String(seconds).padStart(2, '0')}`
}
