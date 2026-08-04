/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Subtle "learn BrowserClaw" strip at the bottom of the ready-state
 * cockpit. A single horizontally-scrolling row of poster tiles that
 * open the shared lightbox, foldable to a one-line handle so it never
 * disturbs the workflow above it. Fold state persists in localStorage.
 * Reuses the onboarding video manifest, tile, and player.
 */

import { ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { ONBOARDING_VIDEOS, type OnboardingVideo } from './cockpit-videos'
import { VideoLightbox } from './VideoLightbox'
import { VideoTile } from './VideoTile'

const COLLAPSED_KEY = 'cockpit.video-rail.collapsed'

function readCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED_KEY) === 'true'
  } catch {
    return false
  }
}

export function VideoRail() {
  const [collapsed, setCollapsed] = useState(readCollapsed)
  const [active, setActive] = useState<OnboardingVideo | null>(null)

  const openVideo = (video: OnboardingVideo) => {
    track(AnalyticsEvent.OnboardingVideoOpened, {
      tileId: video.id,
      span: video.span,
      surface: 'rail',
    })
    setActive(video)
  }

  const toggle = () => {
    setCollapsed((prev) => {
      const next = !prev
      try {
        localStorage.setItem(COLLAPSED_KEY, String(next))
      } catch {}
      return next
    })
  }

  return (
    <section
      className="flex flex-col gap-3 border-border-2 border-t pt-6"
      aria-label="learn browserclaw"
    >
      <button
        type="button"
        onClick={toggle}
        aria-expanded={!collapsed}
        className="flex items-center gap-2 self-start rounded-md text-ink-2 outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-background"
      >
        <ChevronDown
          className={cn(
            'size-4 text-ink-3 transition-transform duration-200 motion-reduce:transition-none',
            collapsed && '-rotate-90',
          )}
        />
        <span className="font-mono text-[11px] uppercase tracking-[0.14em]">
          Learn BrowserClaw
        </span>
        {collapsed && (
          <span className="font-mono text-[11px] text-ink-3 tracking-[0.08em]">
            {ONBOARDING_VIDEOS.length} videos
          </span>
        )}
      </button>

      <div
        className={cn(
          'grid transition-[grid-template-rows] duration-200 ease-out motion-reduce:transition-none',
          collapsed ? 'grid-rows-[0fr]' : 'grid-rows-[1fr]',
        )}
      >
        <div className="overflow-hidden">
          <div className="flex snap-x gap-3 overflow-x-auto pb-1">
            {ONBOARDING_VIDEOS.map((video) => (
              <VideoTile
                key={video.id}
                video={video}
                onPlay={openVideo}
                variant="rail"
              />
            ))}
          </div>
        </div>
      </div>

      <VideoLightbox video={active} onClose={() => setActive(null)} />
    </section>
  )
}
