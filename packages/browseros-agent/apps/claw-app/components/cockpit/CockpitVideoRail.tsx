/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * "Learn BrowserClaw" rail on the cockpit new tab. The lineup comes from PostHog
 * remote config; tiles are posters that open the video on YouTube in a new tab
 * (a YouTube iframe cannot play from the extension origin). Horizontally
 * scrollable, and foldable to a one-line handle whose state persists in
 * chrome.storage so it stays collapsed until the reader expands it again. Hidden
 * entirely when the config lists no videos.
 */

import { ChevronDown, Play } from 'lucide-react'
import { useEffect, useState } from 'react'
import { cn } from '@/lib/utils'
import {
  type CockpitVideo,
  youtubeThumbnailFallback,
} from './cockpit-video-config'
import { useCockpitVideos } from './cockpit-video-config.hooks'
import { cockpitVideosCollapsedStorage } from './cockpit-videos.storage'

function useCollapsed(): [boolean, (value: boolean) => void] {
  const [collapsed, setCollapsedState] = useState(false)
  useEffect(() => {
    let active = true
    const store = cockpitVideosCollapsedStorage()
    // External system: chrome.storage is async, so the initial value and any
    // cross-tab change arrive here rather than in a synchronous initializer.
    void store.getValue().then((value) => {
      if (active) setCollapsedState(value)
    })
    const unwatch = store.watch((value) => setCollapsedState(value ?? false))
    return () => {
      active = false
      unwatch()
    }
  }, [])
  const setCollapsed = (value: boolean) => {
    setCollapsedState(value)
    void cockpitVideosCollapsedStorage().setValue(value)
  }
  return [collapsed, setCollapsed]
}

export function CockpitVideoRail() {
  const { heading, videos } = useCockpitVideos()
  if (videos.length === 0) return null
  return <VideoRail heading={heading} videos={videos} />
}

// Split so the chrome.storage-backed collapse hook only runs once there is a
// rail to show (and never in a video-less render, e.g. under test).
function VideoRail({
  heading,
  videos,
}: {
  heading: string
  videos: CockpitVideo[]
}) {
  const [collapsed, setCollapsed] = useCollapsed()
  return (
    <section
      className="flex flex-col gap-3 border-border-2 border-t pt-6"
      aria-label={heading.toLowerCase()}
    >
      <button
        type="button"
        onClick={() => setCollapsed(!collapsed)}
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
          {heading}
        </span>
        {collapsed && (
          <span className="font-mono text-[11px] text-ink-3 tracking-[0.08em]">
            {videos.length} videos
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
            {videos.map((video) => (
              <VideoCard key={video.id} video={video} />
            ))}
          </div>
        </div>
      </div>
    </section>
  )
}

function VideoCard({ video }: { video: CockpitVideo }) {
  return (
    <a
      href={video.url}
      target="_blank"
      rel="noopener noreferrer"
      aria-label={`Watch on YouTube: ${video.title ?? video.url}`}
      className="group relative aspect-video w-60 shrink-0 snap-start overflow-hidden rounded-2xl border border-border-2 bg-muted text-left outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-background"
    >
      <Poster video={video} />
      <div className="absolute inset-0 bg-gradient-to-t from-black/70 via-black/10 to-transparent" />
      <span className="absolute top-1/2 left-1/2 flex size-9 -translate-x-1/2 -translate-y-1/2 items-center justify-center rounded-full bg-background/90 text-accent shadow-sm ring-1 ring-accent/20 backdrop-blur transition duration-200 group-hover:scale-110 group-hover:ring-accent/40">
        <Play className="size-4 translate-x-[1px] fill-accent" />
      </span>
      {(video.title || video.channel) && (
        <div className="absolute inset-x-0 bottom-0 flex flex-col gap-1 p-3">
          {video.title && (
            <span className="line-clamp-1 font-semibold text-[13px] text-white leading-tight">
              {video.title}
            </span>
          )}
          {video.channel && (
            <span className="font-mono text-[10.5px] text-white/70 uppercase tracking-[0.08em]">
              {video.channel}
            </span>
          )}
        </div>
      )}
    </a>
  )
}

function Poster({ video }: { video: CockpitVideo }) {
  const [src, setSrc] = useState(video.poster)
  return (
    <img
      src={src}
      alt=""
      loading="lazy"
      onError={() => {
        const fallback = youtubeThumbnailFallback(video.id)
        if (src !== fallback) setSrc(fallback)
      }}
      className="absolute inset-0 h-full w-full object-cover transition duration-300 group-hover:scale-[1.03]"
    />
  )
}
