/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * First-run onboarding hero: the recording stays paused behind its poster
 * until the reader starts it with the native video controls. A single focused
 * player reads as "watch this first".
 */

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
  // Playback stays user-driven; mounting onboarding must not start the video.
  return (
    <div className="relative aspect-video w-full overflow-hidden rounded-3xl border border-border-2 bg-black shadow-sm ring-1 ring-foreground/5 md:aspect-auto md:h-full">
      <video
        src={videoUrlFor(video)}
        poster={posterFor(video)}
        playsInline
        controls
        className="h-full w-full object-cover"
        {...tracking}
      />
    </div>
  )
}
