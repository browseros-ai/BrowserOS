/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * A single how-to video tile. Renders a poster facade with a play
 * affordance; the YouTube iframe only mounts when the reader clicks,
 * over in the shared lightbox. The whole tile is one button so the
 * poster, play chip, and caption are a single keyboard target.
 */

import { Play } from 'lucide-react'
import { cn } from '@/lib/utils'
import { type OnboardingVideo, posterFor } from './cockpit-videos'

interface VideoTileProps {
  video: OnboardingVideo
  onPlay: (video: OnboardingVideo) => void
}

export function VideoTile({ video, onPlay }: VideoTileProps) {
  const featured = video.span === 'featured'
  return (
    <button
      type="button"
      onClick={() => onPlay(video)}
      aria-label={`Play: ${video.title}`}
      className={cn(
        'group relative aspect-video overflow-hidden rounded-2xl border border-border-2 bg-muted text-left outline-none md:aspect-auto md:h-full',
        'focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-background',
        featured && 'md:col-span-2 md:row-span-2',
        video.span === 'wide' && 'md:col-span-2',
      )}
    >
      <img
        src={posterFor(video)}
        alt={video.title}
        loading="lazy"
        className="absolute inset-0 h-full w-full object-cover transition duration-300 group-hover:scale-[1.03]"
      />
      <div className="absolute inset-0 bg-gradient-to-t from-black/70 via-black/10 to-transparent" />

      <span
        className={cn(
          'absolute top-1/2 left-1/2 flex -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full bg-background/90 text-accent shadow-sm ring-1 ring-accent/20 backdrop-blur transition duration-200 group-hover:scale-110 group-hover:ring-accent/40',
          featured ? 'size-16' : 'size-11',
        )}
      >
        <Play
          className={cn(
            'translate-x-[1px] fill-accent',
            featured ? 'size-7' : 'size-5',
          )}
        />
      </span>

      <div className="absolute inset-x-0 bottom-0 flex flex-col gap-1 p-4">
        <span
          className={cn(
            'line-clamp-2 font-semibold text-white leading-tight',
            featured ? 'text-lg' : 'text-sm',
          )}
        >
          {video.title}
        </span>
        <span className="font-mono text-[10.5px] text-white/70 uppercase tracking-[0.08em]">
          {video.channel}
        </span>
      </div>
    </button>
  )
}
