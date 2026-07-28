/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * First-run onboarding hero video: one large, focused poster that opens
 * the shared lightbox. A single clip reads as a deliberate "watch this
 * first" rather than a grid of options.
 */

import { Play } from 'lucide-react'
import { useState } from 'react'
import {
  ONBOARDING_VIDEOS,
  type OnboardingVideo,
  posterFor,
} from './cockpit-videos'
import { VideoLightbox } from './VideoLightbox'

export function VideoFeature() {
  const [open, setOpen] = useState<OnboardingVideo | null>(null)
  const video = ONBOARDING_VIDEOS[0]
  if (!video) return null
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(video)}
        aria-label={`Play: ${video.title}`}
        className="group relative aspect-video w-full overflow-hidden rounded-3xl border border-border-2 bg-muted text-left shadow-sm outline-none ring-1 ring-foreground/5 focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-background"
      >
        <img
          src={posterFor(video)}
          alt={video.title}
          className="absolute inset-0 h-full w-full object-cover transition duration-500 group-hover:scale-[1.02]"
        />
        <div className="absolute inset-0 bg-gradient-to-t from-black/75 via-black/20 to-transparent" />

        <span className="absolute top-1/2 left-1/2 flex size-20 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full bg-background/90 text-accent shadow ring-1 ring-accent/25 backdrop-blur transition duration-200 group-hover:scale-110 group-hover:ring-accent/40">
          <Play className="size-8 translate-x-[2px] fill-accent" />
        </span>

        <div className="absolute inset-x-0 bottom-0 flex flex-col gap-2 p-6 md:p-8">
          <span className="max-w-2xl font-semibold text-white text-xl leading-tight md:text-3xl">
            {video.title}
          </span>
          <span className="font-mono text-[11px] text-white/70 uppercase tracking-[0.12em]">
            {video.channel}
          </span>
        </div>
      </button>

      <VideoLightbox video={open} onClose={() => setOpen(null)} />
    </>
  )
}
