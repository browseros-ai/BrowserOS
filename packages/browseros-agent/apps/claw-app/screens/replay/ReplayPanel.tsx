/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Maximize2 } from 'lucide-react'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { PlaybackTransport } from './PlaybackTransport'
import { ReplayPoster, ReplayViewport } from './ReplayViewport'
import type { ReplaySession } from './use-replay-session'

interface ReplayPanelProps {
  session: ReplaySession
  /** The recording is still downloading: hold the layout with a still. */
  loading?: boolean
  /** The still shown in the viewport while loading. */
  posterSrc?: string
  /** Offers full screen from the transport, and from the viewport on hover. */
  onToggleFullScreen?: () => void
  /** This panel is the full-screen player. */
  fullScreen?: boolean
  /**
   * The full-screen player is showing this session, so hold a still in the
   * same frame. Each viewport builds its own rrweb Replayer; a second one would
   * rebuild the page under the overlay for nothing. Unparking remounts it at
   * the shared playback time.
   */
  parked?: boolean
}

/** The replay player: tab strip, recording warnings, viewport and transport. */
export function ReplayPanel({
  session,
  loading = false,
  posterSrc,
  onToggleFullScreen,
  fullScreen = false,
  parked = false,
}: ReplayPanelProps) {
  const {
    replay,
    plan,
    playback,
    state,
    visibleTabId,
    pinnedTabId,
    selectTab,
    resumeFollow,
    seek,
  } = session
  if (!replay) return null

  const track = plan.trackFor(visibleTabId)
  const view = plan.viewFor(visibleTabId)
  const projection = plan.project(visibleTabId, playback.time)

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
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
              className="ml-auto shrink-0 rounded-md bg-accent-tint px-2.5 py-1 font-semibold text-accent-ink text-xs"
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
        <div className="group/viewport relative flex min-h-0 flex-1">
          {loading ? (
            <ReplayPoster
              url={replay.frames.find((frame) => frame.url)?.url ?? replay.site}
              src={posterSrc}
            />
          ) : parked ? (
            <ReplayPoster
              url={plan.urlAt(visibleTabId, playback.time) ?? replay.site}
              src={posterSrc}
            />
          ) : track ? (
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
          {onToggleFullScreen &&
          !fullScreen &&
          !loading &&
          plan.durationSeconds > 0 ? (
            // A pointer shortcut only: the transport's button is the keyboard
            // path, so this one stays out of the tab order.
            <button
              type="button"
              tabIndex={-1}
              aria-label="Full screen"
              onClick={onToggleFullScreen}
              className="pointer-events-none absolute top-12 right-3 z-10 flex items-center gap-1.5 rounded-md bg-ink-deep/90 py-1.5 pr-2.5 pl-2 font-semibold text-[12.5px] text-white leading-4 opacity-0 transition-opacity duration-200 group-hover/viewport:pointer-events-auto group-hover/viewport:opacity-100"
            >
              <Maximize2 aria-hidden className="size-[15px]" />
              Full screen
            </button>
          ) : null}
        </div>
        {loading ? (
          <PlaybackTransport
            disabled
            playback={playback}
            totalSeconds={replay.totalSeconds}
            actions={[]}
            onSeek={seek}
            onToggleFullScreen={onToggleFullScreen}
            fullScreen={fullScreen}
          />
        ) : plan.durationSeconds > 0 ? (
          <PlaybackTransport
            playback={playback}
            totalSeconds={plan.durationSeconds}
            actions={plan.actions}
            onSeek={seek}
            onToggleFullScreen={onToggleFullScreen}
            fullScreen={fullScreen}
          />
        ) : null}
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
