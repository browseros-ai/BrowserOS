/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shared lightbox player for the "Learn BrowserClaw" rail. One <video> lives
 * here and only while a tile is open, so the rail loads as posters and nothing
 * streams until the reader asks for one.
 */

import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { type OnboardingVideo, posterFor, videoUrlFor } from './cockpit-videos'
import { useOnboardingVideoTracking } from './cockpit-videos.hooks'

interface VideoLightboxProps {
  video: OnboardingVideo | null
  onClose: () => void
}

export function VideoLightbox({ video, onClose }: VideoLightboxProps) {
  return (
    <Dialog
      open={video !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <DialogContent className="w-full gap-0 overflow-hidden p-0 sm:max-w-3xl">
        {video && <LightboxPlayer key={video.id} video={video} />}
      </DialogContent>
    </Dialog>
  )
}

function LightboxPlayer({ video }: { video: OnboardingVideo }) {
  const tracking = useOnboardingVideoTracking(video)
  return (
    <>
      <DialogTitle className="sr-only">{video.title}</DialogTitle>
      <div className="aspect-video w-full bg-black">
        <video
          src={videoUrlFor(video)}
          poster={posterFor(video)}
          controls
          autoPlay
          playsInline
          className="h-full w-full"
          {...tracking}
        />
      </div>
      <div className="flex flex-col gap-1 p-5">
        <span className="font-semibold text-base text-ink">{video.title}</span>
        <span className="font-mono text-[11px] text-ink-3 uppercase tracking-[0.08em]">
          {video.channel}
        </span>
      </div>
    </>
  )
}
