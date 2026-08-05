/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * First-run onboarding hero: the recording plays inline and autoplays with
 * sound. A browser that blocks unmuted autoplay (no user gesture yet, and stock
 * Chromium applies this to extension pages too) makes play() reject; we then
 * fall back to a muted autoplay so the demo always starts, and the reader can
 * unmute via the controls. A single focused player reads as "watch this first".
 */

import { useEffect, useRef } from 'react'
import {
  ONBOARDING_VIDEOS,
  type OnboardingVideo,
  posterFor,
  videoUrlFor,
} from './cockpit-videos'
import { useOnboardingVideoTracking } from './cockpit-videos.hooks'

export function VideoFeature() {
  const video = ONBOARDING_VIDEOS[0]
  if (!video) return null
  return <FeatureVideo video={video} />
}

function FeatureVideo({ video }: { video: OnboardingVideo }) {
  const tracking = useOnboardingVideoTracking(video)
  const videoRef = useRef<HTMLVideoElement>(null)
  // External-system side effect: kick off playback and, if the browser blocks
  // unmuted autoplay, retry muted so the demo always starts. Cannot be done
  // declaratively because we need the play() promise to detect the block.
  useEffect(() => {
    const el = videoRef.current
    if (!el) return
    el.play().catch(() => {
      el.muted = true
      el.play().catch(() => {})
    })
  }, [])
  return (
    <div className="relative aspect-video w-full overflow-hidden rounded-3xl border border-border-2 bg-black shadow-sm ring-1 ring-foreground/5 md:aspect-auto md:h-full">
      <video
        ref={videoRef}
        src={videoUrlFor(video)}
        poster={posterFor(video)}
        autoPlay
        playsInline
        controls
        className="h-full w-full object-cover"
        {...tracking}
      />
    </div>
  )
}
