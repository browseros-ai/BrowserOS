/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Top-level page for `/audit/:sessionId/replay`. Reuses the
 * existing scaffold's header / viewport / transport / timeline
 * layout but wires it to real data via `useReplayData`. The page
 * owns three pieces of state:
 *
 *   1. `selectedTabPageId`: which of the agent's tabs is currently
 *      replaying. Defaults to the first tab that has events.
 *   2. `playerHandle`: the imperative interface ReplayViewport
 *      hands back once rrweb-player mounts. We forward
 *      PlaybackTransport's seek/play/pause to it.
 *   3. The scaffold's `usePlayback` clock keeps owning the time
 *      cursor. PlaybackTransport's scrub event fires
 *      `playback.seek(t)` AND `playerHandle.goto(t * 1000)` in
 *      lockstep. The rrweb-player runs silently in the background
 *      since its controller is hidden.
 */

import { ArrowLeft, History } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useLocation } from 'react-router'
import { StatusBadge } from '@/components/cockpit/StatusBadge'
import { Spinner } from '@/components/ui/spinner'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { EventTimeline } from './EventTimeline'
import { PlaybackTransport } from './PlaybackTransport'
import { type ReplayPlayerHandle, ReplayViewport } from './ReplayViewport'
import { buildTabView, EMPTY_TAB_VIEW, useReplayData } from './replay.data'
import { frameIndexAt } from './replay.helpers'
import { usePlayback } from './use-playback'

export function Replay() {
  const { replay, isLoading, navigate } = useReplayData()
  const location = useLocation()
  const [selectedTabPageId, setSelectedTabPageId] = useState<number | null>(
    null,
  )
  const playerHandleRef = useRef<ReplayPlayerHandle | null>(null)

  // Default the tab selector once the data lands.
  useEffect(() => {
    if (selectedTabPageId !== null) return
    if (!replay || replay.tabPageIds.length === 0) return
    setSelectedTabPageId(replay.tabPageIds[0])
  }, [replay, selectedTabPageId])

  // Per-tab view: frames + events + duration scoped to the
  // currently-selected tab, with frames time-shifted to tab-
  // relative t=0. Every panel below (viewport, timeline, scrubber)
  // reads from this and only this. Must be declared BEFORE the
  // isLoading early-return so rules-of-hooks stays honest.
  const perTabView = useMemo(
    () =>
      replay
        ? buildTabView(
            {
              frames: replay.frames,
              eventsForTab: replay.eventsForTab,
              startedAtMs: replay.startedAtMs,
            },
            selectedTabPageId,
          )
        : EMPTY_TAB_VIEW,
    [replay, selectedTabPageId],
  )

  // Tab-scoped clock. Its totalSeconds changes when the operator
  // switches tabs; the seek-to-0 effect below lands the playhead
  // at the start of the new tab's story.
  const playback = usePlayback(perTabView.totalSeconds)

  // When playback's time changes (driven by the scaffold's
  // setInterval clock), forward to the rrweb-player. Without this
  // the player would sit idle while the scrubber + timeline
  // advance on their own.
  useEffect(() => {
    playerHandleRef.current?.goto(playback.time * 1000)
  }, [playback.time])

  // On tab switch, reset the clock to 0 so the operator lands at
  // the tab's start (Option A: per-tab clocks). usePlayback's
  // internal useState would otherwise keep the previous tab's
  // position, which is meaningless in the new tab's timeline.
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentionally scoped to selectedTabPageId; `playback.seek` is a new reference every render and would cause an infinite loop.
  useEffect(() => {
    playback.seek(0)
  }, [selectedTabPageId])

  // Mirror play/pause to the rrweb-player. The player still has
  // its own internal clock for rendering frames between our seek
  // updates, so a coarse play/pause is enough.
  useEffect(() => {
    if (!playerHandleRef.current) return
    if (playback.isPlaying) playerHandleRef.current.play()
    else playerHandleRef.current.pause()
  }, [playback.isPlaying])

  const onPlayerReady = useCallback((handle: ReplayPlayerHandle) => {
    playerHandleRef.current = handle
  }, [])

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
  // Everything below is tab-scoped: frame index, current frame,
  // scrubber ticks, timeline actions. Playback.time is already in
  // tab-relative seconds thanks to the per-tab usePlayback wiring.
  const currentTabFrameIndex = frameIndexAt(perTabView.frames, playback.time)
  const currentTabFrame = perTabView.frames[currentTabFrameIndex]

  const stats: { label: string; value: string }[] = [
    { label: 'Duration', value: replay.duration },
    { label: 'Steps', value: replay.steps },
    { label: 'Approvals', value: replay.approvals },
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
          {replay.tabPageIds.length > 1 && selectedTabPageId !== null && (
            <Tabs
              value={String(selectedTabPageId)}
              onValueChange={(v) => setSelectedTabPageId(Number(v))}
            >
              <TabsList variant="line">
                {replay.tabPageIds.map((id, idx) => (
                  <TabsTrigger key={id} value={String(id)}>
                    Tab {idx + 1}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
          )}
          <ReplayViewport
            site={replay.site}
            frame={currentTabFrame}
            events={perTabView.events}
            onPlayerReady={onPlayerReady}
          />
          <PlaybackTransport
            playback={playback}
            totalSeconds={perTabView.totalSeconds}
            frames={perTabView.frames}
          />
        </div>
        <EventTimeline
          frames={perTabView.frames}
          currentFrameIndex={currentTabFrameIndex}
          currentTime={playback.time}
          onSeek={playback.seek}
        />
      </div>
    </div>
  )
}
