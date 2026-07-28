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
  /** `bento` sizes to the grid cell; `rail` is a fixed-width scroll item. */
  variant?: 'bento' | 'rail'
}

export function VideoTile({
  video,
  onPlay,
  variant = 'bento',
}: VideoTileProps) {
  const featured = variant === 'bento' && video.span === 'featured'
  const rail = variant === 'rail'
  return (
    <button
      type="button"
      onClick={() => onPlay(video)}
      aria-label={`Play: ${video.title}`}
      className={cn(
        'group relative aspect-video overflow-hidden rounded-2xl border border-border-2 bg-muted text-left outline-none',
        'focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-background',
        variant === 'bento' && 'md:aspect-auto md:h-full',
        featured && 'md:col-span-2 md:row-span-2',
        variant === 'bento' && video.span === 'wide' && 'md:col-span-2',
        rail && 'w-60 shrink-0 snap-start',
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
          featured ? 'size-16' : rail ? 'size-9' : 'size-11',
        )}
      >
        <Play
          className={cn(
            'translate-x-[1px] fill-accent',
            featured ? 'size-7' : rail ? 'size-4' : 'size-5',
          )}
        />
      </span>

      <div
        className={cn(
          'absolute inset-x-0 bottom-0 flex flex-col gap-1',
          rail ? 'p-3' : 'p-4',
        )}
      >
        <span
          className={cn(
            'font-semibold text-white leading-tight',
            featured ? 'text-lg' : 'text-sm',
            rail ? 'line-clamp-1' : 'line-clamp-2',
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
