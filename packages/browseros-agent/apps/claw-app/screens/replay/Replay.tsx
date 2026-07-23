/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Session replay orchestration. BrowserClaw owns one global clock and a pure
 * semantic camera; rrweb players are bounded visual sinks for the selected
 * per-tab streams. Manual inspection deliberately switches to an independent
 * local transport without rewriting the preserved session position.
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
import { ReplayViewport } from './ReplayViewport'
import {
  type ReplayData,
  type UseReplayDataResult,
  useReplayData,
} from './replay.data'
import { frameIndexAt } from './replay.helpers'
import {
  createSessionCameraState,
  type SessionCameraState,
  sessionCameraReducer,
} from './session-camera'
import {
  buildSessionReplayPlan,
  projectGlobalTimeToTrack,
  type SessionReplayPlan,
  type TabTrack,
} from './session-replay'
import { usePlayback } from './use-playback'

/** Loads replay data, then keeps the stateful transport scoped to one session. */
export function Replay() {
  const { replay, isLoading, navigate } = useReplayData()
  const location = useLocation()

  if (isLoading || !replay) {
    return (
      <div className="flex h-full flex-1 items-center justify-center bg-bg-canvas text-ink-3">
        <Spinner />
      </div>
    )
  }

  return (
    <ReplaySession
      key={replay.sessionId}
      replay={replay}
      navigate={navigate}
      location={location}
    />
  )
}

interface ReplaySessionProps {
  replay: ReplayData
  navigate: UseReplayDataResult['navigate']
  location: ReturnType<typeof useLocation>
}

function ReplaySession({ replay, navigate, location }: ReplaySessionProps) {
  const plan = useMemo(
    () =>
      buildSessionReplayPlan({
        totalSeconds: replay.totalSeconds,
        frames: replay.frames,
        tabs: replay.tabs,
        eventsForTab: replay.eventsForTab,
        startedAtMs: replay.startedAtMs,
      }),
    [
      replay.eventsForTab,
      replay.frames,
      replay.startedAtMs,
      replay.tabs,
      replay.totalSeconds,
    ],
  )
  const globalPlayback = usePlayback(plan.totalSeconds)
  const [camera, setCamera] = useState<SessionCameraState>(() =>
    createSessionCameraState(plan),
  )
  const [syncKey, setSyncKey] = useState(0)
  const lastSessionIdRef = useRef(replay.sessionId)
  const lastGlobalPlayedRealMsRef = useRef(globalPlayback.playedRealMs)

  /**
   * Live audit polling may append semantic candidates without changing rrweb
   * event identity. Keep the current dwell/camera when its track still exists;
   * only initialize anew for a different session or a newly playable replay.
   */
  useEffect(() => {
    const changedSession = lastSessionIdRef.current !== replay.sessionId
    lastSessionIdRef.current = replay.sessionId
    setCamera((current) => {
      if (changedSession) return createSessionCameraState(plan)
      const activeTrack =
        current.activeTabId === null
          ? undefined
          : plan.tracksByTab.get(current.activeTabId)
      if (
        current.mode === 'inspect'
          ? activeTrack !== undefined
          : activeTrack?.hasFullSnapshot
      ) {
        return current
      }
      return createSessionCameraState(plan)
    })
  }, [plan, replay.sessionId])

  /**
   * The clock exposes cumulative real playing time alongside session seconds.
   * Feeding the delta into the camera avoids a second animation loop and keeps
   * ten seconds of dwell equal at 1x, 2x, and 4x. A final delta charged while
   * pausing is applied before the state's visible playing flag is cleared.
   */
  useEffect(() => {
    const realDeltaMs = Math.max(
      0,
      globalPlayback.playedRealMs - lastGlobalPlayedRealMsRef.current,
    )
    lastGlobalPlayedRealMsRef.current = globalPlayback.playedRealMs
    setCamera((current) => {
      if (current.mode !== 'follow') return current
      const advanced = sessionCameraReducer(plan, current, {
        type: 'tick',
        globalSeconds: globalPlayback.time,
        realDeltaMs,
        playing: globalPlayback.isPlaying || realDeltaMs > 0,
      })
      return advanced.isPlaying === globalPlayback.isPlaying
        ? advanced
        : { ...advanced, isPlaying: globalPlayback.isPlaying }
    })
  }, [
    globalPlayback.isPlaying,
    globalPlayback.playedRealMs,
    globalPlayback.time,
    plan,
  ])

  const inspectedTrack =
    camera.inspectTabId === null
      ? undefined
      : plan.tracksByTab.get(camera.inspectTabId)
  const inspectPlayback = usePlayback(
    camera.mode === 'inspect' && inspectedTrack?.hasFullSnapshot
      ? inspectedTrack.totalSeconds
      : 0,
  )

  const requestViewportSync = useCallback(() => {
    setSyncKey((current) => current + 1)
  }, [])

  const seekGlobal = useCallback(
    (seconds: number) => {
      const applied = globalPlayback.seek(seconds)
      setCamera((current) =>
        sessionCameraReducer(plan, current, {
          type: 'seek',
          globalSeconds: applied,
        }),
      )
      inspectPlayback.seek(0)
      requestViewportSync()
    },
    [globalPlayback.seek, inspectPlayback.seek, plan, requestViewportSync],
  )

  const selectFrame = useCallback(
    (frame: ReplayFrame) => {
      const applied = globalPlayback.seek(frame.t)
      setCamera((current) =>
        sessionCameraReducer(plan, current, {
          type: 'select-frame',
          frame: { ...frame, t: applied },
        }),
      )
      inspectPlayback.seek(0)
      requestViewportSync()
    },
    [globalPlayback.seek, inspectPlayback.seek, plan, requestViewportSync],
  )

  const selectTab = useCallback(
    (value: string) => {
      const tabId = Number(value)
      if (!Number.isSafeInteger(tabId) || !plan.tracksByTab.has(tabId)) return
      if (camera.mode === 'inspect' && camera.inspectTabId === tabId) return
      const preservedGlobalSeconds = globalPlayback.pause()
      inspectPlayback.seek(0)
      setCamera((current) =>
        sessionCameraReducer(plan, current, {
          type: 'inspect',
          tabId,
          globalSeconds: current.resumeGlobalSeconds ?? preservedGlobalSeconds,
        }),
      )
      requestViewportSync()
    },
    [
      camera.inspectTabId,
      camera.mode,
      globalPlayback.pause,
      inspectPlayback.seek,
      plan,
      requestViewportSync,
    ],
  )

  const resumeSession = useCallback(() => {
    const resumed = sessionCameraReducer(plan, camera, { type: 'resume' })
    globalPlayback.seek(resumed.globalSeconds)
    inspectPlayback.seek(0)
    setCamera(resumed)
    requestViewportSync()
  }, [
    camera,
    globalPlayback.seek,
    inspectPlayback.seek,
    plan,
    requestViewportSync,
  ])

  const toggleGlobalPlayback = useCallback(() => {
    if (plan.totalSeconds > 0 && globalPlayback.time >= plan.totalSeconds) {
      setCamera((current) =>
        sessionCameraReducer(plan, current, { type: 'restart' }),
      )
      requestViewportSync()
    }
    globalPlayback.togglePlay()
  }, [
    globalPlayback.time,
    globalPlayback.togglePlay,
    plan,
    requestViewportSync,
  ])

  const sessionPlayback = useMemo(
    () => ({ ...globalPlayback, togglePlay: toggleGlobalPlayback }),
    [globalPlayback, toggleGlobalPlayback],
  )

  const activeTrack = trackForCamera(plan, camera)
  const pendingTrack =
    camera.mode === 'follow' && camera.pendingTabId !== null
      ? plan.tracksByTab.get(camera.pendingTabId)
      : undefined
  const activeSeconds =
    camera.mode === 'inspect'
      ? inspectPlayback.time
      : activeTrack
        ? projectGlobalTimeToTrack(activeTrack, globalPlayback.time)
        : 0
  const pendingSeconds = pendingTrack
    ? projectGlobalTimeToTrack(pendingTrack, globalPlayback.time)
    : 0
  const activeFrame = frameAt(activeTrack?.frames ?? [], activeSeconds)
  const timelineSeconds =
    camera.mode === 'inspect'
      ? (camera.resumeGlobalSeconds ?? camera.globalSeconds)
      : globalPlayback.time
  const currentTimelineFrameIndex =
    replay.frames.length === 0
      ? -1
      : frameIndexAt(replay.frames, timelineSeconds)
  const transport =
    camera.mode === 'inspect' ? inspectPlayback : sessionPlayback
  const transportTotalSeconds =
    camera.mode === 'inspect'
      ? (activeTrack?.totalSeconds ?? 0)
      : plan.totalSeconds
  const transportFrames =
    camera.mode === 'inspect' ? (activeTrack?.frames ?? []) : replay.frames
  const transportLabel =
    camera.mode === 'inspect'
      ? `${tabLabel(plan, camera.activeTabId)} playback`
      : 'Session playback'
  const seekTransport =
    camera.mode === 'inspect'
      ? (seconds: number) => {
          inspectPlayback.seek(seconds)
          requestViewportSync()
        }
      : seekGlobal

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
          <div className="flex min-h-8 items-center justify-between gap-3">
            {replay.tabs.length > 1 && camera.activeTabId !== null ? (
              <Tabs
                value={camera.activeTabId.toString()}
                onValueChange={selectTab}
              >
                <TabsList variant="line">
                  {replay.tabs.map(({ tabId }, index) => {
                    const pinned =
                      camera.mode === 'inspect' && camera.activeTabId === tabId
                    return (
                      <TabsTrigger
                        key={tabId}
                        value={tabId.toString()}
                        aria-label={`Inspect Tab ${index + 1}`}
                      >
                        Tab {index + 1}
                        {pinned ? ' · Pinned' : ''}
                      </TabsTrigger>
                    )
                  })}
                </TabsList>
              </Tabs>
            ) : (
              <span />
            )}
            <div className="flex items-center gap-2">
              <span role="status" className="font-semibold text-ink-3 text-xs">
                {camera.mode === 'follow'
                  ? 'Following session'
                  : `Inspecting ${tabLabel(plan, camera.activeTabId)}`}
              </span>
              {camera.mode === 'inspect' && (
                <button
                  type="button"
                  onClick={resumeSession}
                  className="rounded-lg border border-border-2 bg-card px-2.5 py-1 font-semibold text-accent-ink text-xs hover:bg-accent-tint"
                >
                  Resume session
                </button>
              )}
            </div>
          </div>

          {activeTrack?.incompleteUntilMs !== null &&
            activeTrack?.incompleteUntilMs !== undefined && (
              <div
                role="status"
                className="rounded-lg border border-amber/30 bg-amber-tint px-3 py-2 font-medium text-ink-2 text-xs"
              >
                Recording incomplete — playback starts at{' '}
                {formatIncompleteOffset(activeTrack.incompleteUntilMs)}
              </div>
            )}
          {activeTrack?.incompleteUntilMs === null &&
            activeTrack.knownIncomplete && (
              <div
                role="status"
                className="rounded-lg border border-amber/30 bg-amber-tint px-3 py-2 font-medium text-ink-2 text-xs"
              >
                Recording incomplete — this replay contains a known gap
              </div>
            )}

          {activeTrack?.hasFullSnapshot ? (
            <>
              <ReplayViewport
                site={replay.site}
                frame={activeFrame}
                activeTrack={activeTrack}
                standbyTrack={pendingTrack ?? null}
                activeTimeMs={activeSeconds * 1000}
                standbyTimeMs={pendingSeconds * 1000}
                isPlaying={transport.isPlaying}
                speed={transport.speed}
                syncKey={syncKey}
                mode={camera.mode}
              />
              <PlaybackTransport
                playback={transport}
                totalSeconds={transportTotalSeconds}
                frames={transportFrames}
                onSeek={seekTransport}
                transportLabel={transportLabel}
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

function trackForCamera(
  plan: SessionReplayPlan,
  camera: SessionCameraState,
): TabTrack | undefined {
  return camera.activeTabId === null
    ? undefined
    : plan.tracksByTab.get(camera.activeTabId)
}

function frameAt(
  frames: readonly ReplayFrame[],
  seconds: number,
): ReplayFrame | undefined {
  return frames.length === 0 ? undefined : frames[frameIndexAt(frames, seconds)]
}

function tabLabel(plan: SessionReplayPlan, tabId: number | null): string {
  const index = plan.tracks.findIndex((track) => track.tabId === tabId)
  return index === -1 ? 'tab' : `Tab ${index + 1}`
}

function formatIncompleteOffset(milliseconds: number): string {
  const totalSeconds = Math.floor(milliseconds / 1000)
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}:${String(seconds).padStart(2, '0')}`
}
