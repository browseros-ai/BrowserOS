/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shared lightbox player for the how-to bento. Exactly one YouTube
 * iframe lives here and only while a video is open, so the grid loads
 * as posters and no video streams until the reader asks for one.
 */

import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { embedUrlFor, type OnboardingVideo } from './cockpit-videos'

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
        {video && (
          <>
            <DialogTitle className="sr-only">{video.title}</DialogTitle>
            <div className="aspect-video w-full bg-black">
              <iframe
                key={video.id}
                src={embedUrlFor(video)}
                title={video.title}
                allow="autoplay; encrypted-media; picture-in-picture; web-share"
                allowFullScreen
                className="h-full w-full border-0"
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
