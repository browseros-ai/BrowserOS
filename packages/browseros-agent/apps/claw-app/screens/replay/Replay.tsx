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
import {
  buildTabView,
  EMPTY_TAB_VIEW,
  type ReplayData,
  type TabView,
  useReplayData,
} from './replay.data'
import { frameIndexAt } from './replay.helpers'
import { usePlayback } from './use-playback'

const CHAPTER_TRANSITION_MS = 1_000

interface ChapterTransition {
  title: string
  skipped: string | null
}

/** Renders the audit replay page and syncs rrweb playback to the transport UI. */
export function Replay() {
  const { replay, isLoading, navigate } = useReplayData()
  const location = useLocation()
  const [selectedTabId, setSelectedTabId] = useState<number | null>(null)
  const [transition, setTransition] = useState<ChapterTransition | null>(null)
  const [sequenceComplete, setSequenceComplete] = useState(false)
  const playerHandleRef = useRef<ReplayPlayerHandle | null>(null)
  const playbackTimeRef = useRef(0)
  const playbackSpeedRef = useRef(1)
  const playbackIsPlayingRef = useRef(true)
  const pendingTabSeekRef = useRef<number | null>(null)
  const pendingTabPlaybackRef = useRef<'pause' | 'play'>('pause')
  const transitionTimerRef = useRef<number | null>(null)
  const transitionTokenRef = useRef(0)
  const autoStartedSessionRef = useRef<string | null>(null)

  const tabViewInput = useMemo(
    () =>
      replay
        ? {
            frames: replay.frames,
            tabs: replay.tabs,
            eventsForTab: replay.eventsForTab,
            startedAtMs: replay.startedAtMs,
          }
        : null,
    [replay],
  )
  const chapterViews = useMemo(
    () =>
      replay && tabViewInput
        ? replay.tabs.map(({ tabId }) => buildTabView(tabViewInput, tabId))
        : [],
    [replay, tabViewInput],
  )
  const selectedTabIndex =
    replay?.tabs.findIndex(({ tabId }) => tabId === selectedTabId) ?? -1
  const perTabView =
    selectedTabIndex >= 0
      ? (chapterViews[selectedTabIndex] ?? EMPTY_TAB_VIEW)
      : EMPTY_TAB_VIEW
  const playableChapterIndices = useMemo(
    () =>
      chapterViews.flatMap((view, index) =>
        isPlayableChapter(view) ? [index] : [],
      ),
    [chapterViews],
  )

  const playbackTotalSeconds = isPlayableChapter(perTabView)
    ? perTabView.totalSeconds
    : 0
  const playback = usePlayback(playbackTotalSeconds)
  const playbackPlayRef = useRef(playback.play)

  useEffect(() => {
    playbackPlayRef.current = playback.play
  }, [playback.play])

  const invalidateTransition = useCallback(() => {
    transitionTokenRef.current += 1
    const timer = transitionTimerRef.current
    transitionTimerRef.current = null
    if (timer !== null) window.clearTimeout(timer)
  }, [])

  const cancelTransition = useCallback(() => {
    invalidateTransition()
    setTransition(null)
  }, [invalidateTransition])

  const scheduleTransition = useCallback(
    (next: ChapterTransition, onFinished: () => void) => {
      invalidateTransition()
      const token = transitionTokenRef.current
      setTransition(next)
      transitionTimerRef.current = window.setTimeout(() => {
        if (transitionTokenRef.current !== token) return
        transitionTimerRef.current = null
        setTransition(null)
        onFinished()
      }, CHAPTER_TRANSITION_MS)
    },
    [invalidateTransition],
  )

  useEffect(
    () => () => {
      invalidateTransition()
    },
    [invalidateTransition],
  )

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

  const seekTo = useCallback(
    (seconds: number) => {
      const next = playback.seek(seconds)
      playbackTimeRef.current = next
      playbackIsPlayingRef.current = false
      playerHandleRef.current?.seek(next * 1000)
    },
    [playback.seek],
  )

  // biome-ignore lint/correctness/useExhaustiveDependencies: tab changes must flush pending seeks even when consecutive tabs have the same duration.
  useEffect(() => {
    const pendingSeconds = pendingTabSeekRef.current
    if (pendingSeconds === null) return
    const pendingPlayback = pendingTabPlaybackRef.current
    pendingTabSeekRef.current = null
    seekTo(pendingSeconds)
    if (pendingPlayback === 'play') {
      playbackTimeRef.current = pendingSeconds
      playbackIsPlayingRef.current = true
      playback.play()
    }
  }, [seekTo, selectedTabId])

  useEffect(() => {
    if (!replay) return
    const firstTab = replay.tabs[0]
    if (!firstTab) {
      if (selectedTabId !== null) {
        cancelTransition()
        pendingTabSeekRef.current = 0
        setSelectedTabId(null)
      }
      return
    }

    const selectedTab = replay.tabs.find((tab) => tab.tabId === selectedTabId)
    const firstPlayableIndex = playableChapterIndices[0]
    if (firstPlayableIndex === undefined) {
      if (!selectedTab) setSelectedTabId(firstTab.tabId)
      return
    }

    if (autoStartedSessionRef.current !== replay.sessionId) {
      autoStartedSessionRef.current = replay.sessionId
      if (firstPlayableIndex > 0) {
        playback.pause()
        playbackIsPlayingRef.current = false
        playerHandleRef.current?.pause()
        pendingTabSeekRef.current = 0
        pendingTabPlaybackRef.current = 'pause'
        setSelectedTabId(
          replay.tabs[firstPlayableIndex]?.tabId ?? firstTab.tabId,
        )
        scheduleTransition(
          {
            title: `Starting replay → Opening ${chapterName(replay, firstPlayableIndex)}`,
            skipped: skippedChapterSummary(
              Array.from({ length: firstPlayableIndex }, (_, index) => index),
            ),
          },
          () => {
            playbackIsPlayingRef.current = true
            playbackPlayRef.current()
          },
        )
        return
      }
    }

    if (!selectedTab) {
      if (selectedTabId !== null) {
        pendingTabSeekRef.current = 0
        pendingTabPlaybackRef.current = 'pause'
      }
      setSelectedTabId(firstTab.tabId)
    }
  }, [
    cancelTransition,
    playableChapterIndices,
    playback.pause,
    replay,
    scheduleTransition,
    selectedTabId,
  ])

  const selectTab = useCallback(
    (value: string) => {
      const tabId = Number(value)
      if (!replay || !Number.isSafeInteger(tabId)) return
      cancelTransition()
      setSequenceComplete(false)
      playback.pause()
      playbackIsPlayingRef.current = false
      playerHandleRef.current?.pause()
      if (tabId === selectedTabId) {
        seekTo(0)
        return
      }
      pendingTabSeekRef.current = 0
      pendingTabPlaybackRef.current = 'pause'
      setSelectedTabId(tabId)
    },
    [cancelTransition, playback.pause, replay, seekTo, selectedTabId],
  )

  const selectFrame = useCallback(
    (frame: ReplayFrame) => {
      if (transition) return
      seekTo(frame.t)
    },
    [seekTo, transition],
  )

  const onPlayerReady = useCallback((handle: ReplayPlayerHandle | null) => {
    playerHandleRef.current = handle
    if (!handle) return
    const ms = playbackTimeRef.current * 1000
    handle.setSpeed(playbackSpeedRef.current)
    handle.seek(ms)
    if (playbackIsPlayingRef.current) handle.play(ms)
  }, [])

  const advanceChapter = useCallback(() => {
    if (!replay || selectedTabIndex < 0) return
    playback.pause()
    playbackIsPlayingRef.current = false
    playerHandleRef.current?.pause()

    const nextPlayableIndex = playableChapterIndices.find(
      (index) => index > selectedTabIndex,
    )
    if (nextPlayableIndex !== undefined) {
      const skippedIndices = Array.from(
        { length: nextPlayableIndex - selectedTabIndex - 1 },
        (_, offset) => selectedTabIndex + offset + 1,
      )
      setSequenceComplete(false)
      pendingTabSeekRef.current = 0
      pendingTabPlaybackRef.current = 'pause'
      setSelectedTabId(replay.tabs[nextPlayableIndex]?.tabId ?? null)
      scheduleTransition(
        {
          title: `Tab ${selectedTabIndex + 1} complete → Opening ${chapterName(replay, nextPlayableIndex)}`,
          skipped: skippedChapterSummary(skippedIndices),
        },
        () => {
          playbackTimeRef.current = 0
          playbackIsPlayingRef.current = true
          playbackPlayRef.current()
        },
      )
      return
    }

    const trailingSkippedIndices = Array.from(
      { length: replay.tabs.length - selectedTabIndex - 1 },
      (_, offset) => selectedTabIndex + offset + 1,
    )
    const finishSequence = () => {
      setSequenceComplete(true)
    }
    if (trailingSkippedIndices.length > 0) {
      scheduleTransition(
        {
          title: `Tab ${selectedTabIndex + 1} complete → Replay complete`,
          skipped: skippedChapterSummary(trailingSkippedIndices),
        },
        finishSequence,
      )
    } else {
      finishSequence()
    }
  }, [
    playableChapterIndices,
    playback.pause,
    replay,
    scheduleTransition,
    selectedTabIndex,
  ])

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
      if (keepGoing) {
        rafId = window.requestAnimationFrame(sync)
      } else {
        advanceChapter()
      }
    }
    rafId = window.requestAnimationFrame(sync)
    return () => {
      active = false
      window.cancelAnimationFrame(rafId)
    }
  }, [
    advanceChapter,
    playback.isPlaying,
    playback.syncFromPlayer,
    playbackTotalSeconds,
  ])

  const toggleSequencePlayback = useCallback(() => {
    if (transition) return
    if (!sequenceComplete) {
      playback.togglePlay()
      return
    }

    const firstPlayableIndex = playableChapterIndices[0]
    if (!replay || firstPlayableIndex === undefined) return
    cancelTransition()
    setSequenceComplete(false)
    playback.pause()
    playbackIsPlayingRef.current = false
    playerHandleRef.current?.pause()
    const firstPlayableTabId = replay.tabs[firstPlayableIndex]?.tabId ?? null
    if (firstPlayableTabId === selectedTabId) {
      seekTo(0)
      playbackIsPlayingRef.current = true
      playback.play()
      return
    }
    pendingTabSeekRef.current = 0
    pendingTabPlaybackRef.current = 'play'
    setSelectedTabId(firstPlayableTabId)
  }, [
    cancelTransition,
    playableChapterIndices,
    playback,
    replay,
    seekTo,
    selectedTabId,
    sequenceComplete,
    transition,
  ])

  const transportPlayback = useMemo(
    () => ({ ...playback, togglePlay: toggleSequencePlayback }),
    [playback, toggleSequencePlayback],
  )
  const seekFromTransport = useCallback(
    (seconds: number) => {
      if (!transition) seekTo(seconds)
    },
    [seekTo, transition],
  )

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
          {selectedTabIndex >= 0 && (
            <div className="flex min-h-9 items-center gap-3">
              {replay.tabs.length > 1 && selectedTabId !== null && (
                <Tabs
                  value={selectedTabId.toString()}
                  onValueChange={selectTab}
                >
                  <TabsList variant="line">
                    {replay.tabs.map(({ tabId }, idx) => (
                      <TabsTrigger key={tabId} value={tabId.toString()}>
                        Tab {idx + 1}
                      </TabsTrigger>
                    ))}
                  </TabsList>
                </Tabs>
              )}
              <span
                data-chapter-progress
                className="ml-auto shrink-0 font-semibold text-ink-3 text-xs"
              >
                Tab {selectedTabIndex + 1} of {replay.tabs.length}
              </span>
              {sequenceComplete && (
                <span
                  role="status"
                  className="shrink-0 rounded-full bg-accent-tint px-2.5 py-1 font-semibold text-accent-ink text-xs"
                >
                  Replay complete
                </span>
              )}
            </div>
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
            perTabView.knownIncomplete && (
              <div
                role="status"
                className="rounded-lg border border-amber/30 bg-amber-tint px-3 py-2 font-medium text-ink-2 text-xs"
              >
                Recording incomplete — this replay contains a known gap
              </div>
            )}
          <div className="relative flex min-h-0 flex-1 flex-col gap-3">
            <div className="flex min-h-0 flex-1">
              {isPlayableChapter(perTabView) ? (
                <ReplayViewport
                  site={replay.site}
                  frame={currentTabFrame}
                  events={perTabView.events}
                  onPlayerReady={onPlayerReady}
                />
              ) : (
                <div className="flex flex-1 items-center justify-center rounded-2xl border border-border-2 bg-card text-ink-3 text-sm shadow-sm">
                  No visual recording for this tab
                </div>
              )}
            </div>
            {isPlayableChapter(perTabView) && (
              <PlaybackTransport
                playback={transportPlayback}
                totalSeconds={perTabView.totalSeconds}
                frames={perTabView.frames}
                onSeek={seekFromTransport}
              />
            )}
            {transition && (
              <div
                role="status"
                data-chapter-transition
                className="absolute inset-0 z-20 flex flex-col items-center justify-center rounded-2xl bg-ink-deep/95 px-8 text-center shadow-sm"
              >
                <div className="font-semibold text-base text-white">
                  {transition.title}
                </div>
                {transition.skipped && (
                  <div className="mt-2 text-sm text-white/70">
                    {transition.skipped}
                  </div>
                )}
              </div>
            )}
          </div>
        </div>
        <EventTimeline
          frames={perTabView.frames}
          currentFrameIndex={currentTabFrameIndex}
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

function isPlayableChapter(view: TabView): boolean {
  return (
    view.hasFullSnapshot && view.events.length >= 2 && view.totalSeconds > 0
  )
}

function chapterName(replay: ReplayData, chapterIndex: number): string {
  const tabId = replay.tabs[chapterIndex]?.tabId
  const tabUrl = replay.frames.find((frame) => frame.tabId === tabId)?.url
  return `Tab ${chapterIndex + 1} · ${displayHost(tabUrl ?? replay.site)}`
}

function displayHost(value: string): string {
  try {
    const url = value.includes('://')
      ? new URL(value)
      : new URL(`https://${value}`)
    return url.hostname || value
  } catch {
    return value
  }
}

function skippedChapterSummary(indices: readonly number[]): string | null {
  if (indices.length === 0) return null
  if (indices.length === 1) {
    return `Skipped Tab ${(indices[0] ?? 0) + 1} — no visual recording`
  }
  return `Skipped Tabs ${(indices[0] ?? 0) + 1}–${(indices.at(-1) ?? 0) + 1} — no visual recordings`
}
