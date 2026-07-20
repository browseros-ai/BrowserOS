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
import type { ReplayFrame } from '@/modules/api/replay.hooks'
import { EventTimeline } from './EventTimeline'
import { PlaybackTransport } from './PlaybackTransport'
import { type ReplayPlayerHandle, ReplayViewport } from './ReplayViewport'
import { buildTabView, EMPTY_TAB_VIEW, useReplayData } from './replay.data'
import { frameIndexAt } from './replay.helpers'
import { tabSeekForFrame } from './tab-view'
import { usePlayback } from './use-playback'

/** Renders the audit replay page and syncs rrweb playback to the transport UI. */
export function Replay() {
  const { replay, isLoading, navigate } = useReplayData()
  const location = useLocation()
  const [selectedTabId, setSelectedTabId] = useState<number | null>(null)
  const [selectedDocumentId, setSelectedDocumentId] = useState<string | null>(
    null,
  )
  const playerHandleRef = useRef<ReplayPlayerHandle | null>(null)
  const playbackTimeRef = useRef(0)
  const playbackSpeedRef = useRef(1)
  const playbackIsPlayingRef = useRef(true)
  const pendingSegmentSeekRef = useRef<number | null>(null)

  useEffect(() => {
    if (!replay) return
    const firstTab = replay.tabs[0]
    if (!firstTab) {
      if (selectedTabId !== null || selectedDocumentId !== null) {
        pendingSegmentSeekRef.current = 0
        setSelectedTabId(null)
        setSelectedDocumentId(null)
      }
      return
    }
    const selectedTab = replay.tabs.find((tab) => tab.tabId === selectedTabId)
    if (!selectedTab) {
      setSelectedTabId(firstTab.tabId)
      setSelectedDocumentId(firstTab.segments[0]?.documentId ?? null)
      return
    }
    if (
      !selectedTab.segments.some(
        (segment) => segment.documentId === selectedDocumentId,
      )
    ) {
      setSelectedDocumentId(selectedTab.segments[0]?.documentId ?? null)
    }
  }, [replay, selectedDocumentId, selectedTabId])

  const tabViewInput = useMemo(
    () =>
      replay
        ? {
            frames: replay.frames,
            tabs: replay.tabs,
            eventsForDocument: replay.eventsForDocument,
            startedAtMs: replay.startedAtMs,
          }
        : null,
    [replay],
  )
  const perTabView = useMemo(
    () =>
      tabViewInput
        ? buildTabView(tabViewInput, selectedTabId, selectedDocumentId)
        : EMPTY_TAB_VIEW,
    [selectedDocumentId, selectedTabId, tabViewInput],
  )

  const playbackTotalSeconds = perTabView.hasFullSnapshot
    ? perTabView.totalSeconds
    : 0
  const playback = usePlayback(playbackTotalSeconds)

  useEffect(() => {
    playbackTimeRef.current = playback.time
  }, [playback.time])

  useEffect(() => {
    playbackSpeedRef.current = playback.speed
    playerHandleRef.current?.setSpeed(playback.speed)
  }, [playback.speed])

  useEffect(() => {
    playbackIsPlayingRef.current = playback.isPlaying
  }, [playback.isPlaying])

  useEffect(() => {
    if (!playerHandleRef.current) return
    if (playback.isPlaying) {
      playerHandleRef.current.play(playbackTimeRef.current * 1000)
    } else {
      playerHandleRef.current.pause()
    }
  }, [playback.isPlaying])

  useEffect(() => {
    if (!playback.isPlaying || playbackTotalSeconds === 0) return
    let rafId = 0
    let active = true
    const sync = () => {
      if (!active) return
      const handle = playerHandleRef.current
      const keepGoing = handle
        ? playback.syncFromPlayer(handle.getCurrentTime() / 1000)
        : true
      if (keepGoing) rafId = window.requestAnimationFrame(sync)
    }
    rafId = window.requestAnimationFrame(sync)
    return () => {
      active = false
      window.cancelAnimationFrame(rafId)
    }
  }, [playback.isPlaying, playback.syncFromPlayer, playbackTotalSeconds])

  const seekTo = useCallback(
    (seconds: number) => {
      const next = playback.seek(seconds)
      playbackTimeRef.current = next
      playbackIsPlayingRef.current = false
      playerHandleRef.current?.seek(next * 1000)
    },
    [playback.seek],
  )

  // biome-ignore lint/correctness/useExhaustiveDependencies: target changes must flush pending seeks even when both targets have the same duration.
  useEffect(() => {
    const pendingSeconds = pendingSegmentSeekRef.current
    if (pendingSeconds === null) return
    pendingSegmentSeekRef.current = null
    seekTo(pendingSeconds)
  }, [seekTo, selectedDocumentId, selectedTabId])

  const selectTab = useCallback(
    (value: string) => {
      const tabId = Number(value)
      if (!replay || !Number.isSafeInteger(tabId)) return
      if (tabId === selectedTabId) {
        seekTo(0)
        return
      }
      pendingSegmentSeekRef.current = 0
      const tab = replay.tabs.find((candidate) => candidate.tabId === tabId)
      setSelectedTabId(tabId)
      setSelectedDocumentId(tab?.segments[0]?.documentId ?? null)
    },
    [replay, seekTo, selectedTabId],
  )

  const selectDocument = useCallback(
    (documentId: string) => {
      if (documentId === selectedDocumentId) {
        seekTo(0)
        return
      }
      pendingSegmentSeekRef.current = 0
      setSelectedDocumentId(documentId)
    },
    [seekTo, selectedDocumentId],
  )

  const selectFrame = useCallback(
    (frame: ReplayFrame) => {
      if (!tabViewInput) return
      const tabSeek = tabSeekForFrame(
        tabViewInput,
        selectedTabId,
        selectedDocumentId,
        frame,
      )
      if (
        tabSeek.tabId !== null &&
        (tabSeek.tabId !== selectedTabId ||
          tabSeek.documentId !== selectedDocumentId)
      ) {
        pendingSegmentSeekRef.current = tabSeek.seconds
        setSelectedTabId(tabSeek.tabId)
        setSelectedDocumentId(tabSeek.documentId)
        return
      }
      seekTo(tabSeek.seconds)
    },
    [seekTo, selectedDocumentId, selectedTabId, tabViewInput],
  )

  const onPlayerReady = useCallback((handle: ReplayPlayerHandle | null) => {
    playerHandleRef.current = handle
    if (!handle) return
    const ms = playbackTimeRef.current * 1000
    handle.setSpeed(playbackSpeedRef.current)
    handle.seek(ms)
    if (playbackIsPlayingRef.current) handle.play(ms)
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
  const currentTabFrameIndex = frameIndexAt(perTabView.frames, playback.time)
  const currentTabFrame = perTabView.frames[currentTabFrameIndex]
  const currentTimelineFrameIndex =
    currentTabFrame?.dispatchId !== undefined
      ? replay.frames.findIndex(
          (frame) => frame.dispatchId === currentTabFrame.dispatchId,
        )
      : -1
  const selectedTab = replay.tabs.find((tab) => tab.tabId === selectedTabId)

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
          {replay.tabs.length > 1 && selectedTabId !== null && (
            <Tabs value={selectedTabId.toString()} onValueChange={selectTab}>
              <TabsList variant="line">
                {replay.tabs.map(({ tabId }, idx) => (
                  <TabsTrigger key={tabId} value={tabId.toString()}>
                    Tab {idx + 1}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
          )}
          {(selectedTab?.segments.length ?? 0) > 1 &&
            selectedDocumentId !== null && (
              <Tabs value={selectedDocumentId} onValueChange={selectDocument}>
                <TabsList variant="line">
                  {selectedTab?.segments.map((segment, idx) => (
                    <TabsTrigger
                      key={segment.documentId}
                      value={segment.documentId}
                    >
                      Navigation {idx + 1}
                    </TabsTrigger>
                  ))}
                </TabsList>
              </Tabs>
            )}
          {perTabView.incompleteUntilMs !== null && (
            <div
              role="status"
              className="rounded-lg border border-amber/30 bg-amber-tint px-3 py-2 font-medium text-ink-2 text-xs"
            >
              Recording incomplete — playback starts at{' '}
              {formatIncompleteOffset(perTabView.incompleteUntilMs)}
            </div>
          )}
          {perTabView.incompleteUntilMs === null &&
            (perTabView.knownIncomplete || replay.complete === false) && (
              <div
                role="status"
                className="rounded-lg border border-amber/30 bg-amber-tint px-3 py-2 font-medium text-ink-2 text-xs"
              >
                Recording incomplete — this replay contains a known gap
              </div>
            )}
          {perTabView.hasFullSnapshot ? (
            <>
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
                onSeek={seekTo}
              />
            </>
          ) : (
            <div className="flex flex-1 items-center justify-center rounded-2xl border border-border-2 bg-card text-ink-3 text-sm shadow-sm">
              No visual recording for this tab
            </div>
          )}
        </div>
        <EventTimeline
          frames={replay.frames}
          currentFrameIndex={currentTimelineFrameIndex}
          onSelectFrame={selectFrame}
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
