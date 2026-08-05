/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { type SyntheticEvent, useRef } from 'react'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import type { OnboardingVideo } from './cockpit-videos'

/**
 * PostHog playback handlers for an onboarding video element. The first play
 * (autoplay or the first press) reads as "played", a play after a pause as
 * "resumed", a pause before the end as "paused", and reaching the end as
 * "completed". The started flag lives per element, so remounting (a lightbox
 * re-open, keyed by tile) re-arms "played".
 */
export function useOnboardingVideoTracking(video: OnboardingVideo) {
  const started = useRef(false)
  const meta = { tileId: video.id, span: video.span }
  return {
    onPlay: () => {
      if (started.current) {
        track(AnalyticsEvent.OnboardingVideoResumed, meta)
        return
      }
      started.current = true
      track(AnalyticsEvent.OnboardingVideoPlayed, meta)
    },
    onPause: (event: SyntheticEvent<HTMLVideoElement>) => {
      // Reaching the end also fires pause; only a mid-play pause is a real one.
      if (event.currentTarget.ended) return
      track(AnalyticsEvent.OnboardingVideoPaused, meta)
    },
    onEnded: () => {
      track(AnalyticsEvent.OnboardingVideoCompleted, meta)
    },
  }
}
