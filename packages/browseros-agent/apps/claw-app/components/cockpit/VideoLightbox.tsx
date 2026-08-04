/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shared lightbox player for the how-to bento. Exactly one YouTube
 * iframe lives here and only while a video is open, so the grid loads
 * as posters and no video streams until the reader asks for one.
 */

import { useRef } from 'react'
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { type OnboardingVideo, posterFor, videoUrlFor } from './cockpit-videos'

interface VideoLightboxProps {
  video: OnboardingVideo | null
  onClose: () => void
}

function videoMeta(video: OnboardingVideo) {
  return { tileId: video.id, span: video.span }
}

export function VideoLightbox({ video, onClose }: VideoLightboxProps) {
  // `onPlay` fires both on the initial autoplay and on every resume after a
  // pause. Track the tile whose playback has started so the first play reads as
  // "played" and later plays as "resumed"; cleared on close so a re-open starts
  // fresh.
  const startedTileId = useRef<string | null>(null)
  return (
    <Dialog
      open={video !== null}
      onOpenChange={(open) => {
        if (!open) {
          startedTileId.current = null
          onClose()
        }
      }}
    >
      <DialogContent className="w-full gap-0 overflow-hidden p-0 sm:max-w-3xl">
        {video && (
          <>
            <DialogTitle className="sr-only">{video.title}</DialogTitle>
            <div className="aspect-video w-full bg-black">
              {/* biome-ignore lint/a11y/useMediaCaption: onboarding demos ship without a captions track yet */}
              <video
                key={video.id}
                src={videoUrlFor(video)}
                poster={posterFor(video)}
                controls
                autoPlay
                playsInline
                className="h-full w-full"
                onPlay={() => {
                  if (startedTileId.current === video.id) {
                    track(
                      AnalyticsEvent.OnboardingVideoResumed,
                      videoMeta(video),
                    )
                    return
                  }
                  startedTileId.current = video.id
                  track(AnalyticsEvent.OnboardingVideoPlayed, videoMeta(video))
                }}
                onPause={(event) => {
                  // Reaching the end also fires pause; only a mid-play pause is
                  // a real "paused".
                  if (event.currentTarget.ended) return
                  track(AnalyticsEvent.OnboardingVideoPaused, videoMeta(video))
                }}
                onEnded={() =>
                  track(
                    AnalyticsEvent.OnboardingVideoCompleted,
                    videoMeta(video),
                  )
                }
              />
            </div>
            <div className="flex flex-col gap-1 p-5">
              <span className="font-semibold text-base text-ink">
                {video.title}
              </span>
              <span className="font-mono text-[11px] text-ink-3 uppercase tracking-[0.08em]">
                {video.channel}
              </span>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
