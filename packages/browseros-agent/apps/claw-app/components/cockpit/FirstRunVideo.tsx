/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useEffect, useRef, useState } from 'react'

const CDN_BASE_URL = 'https://cdn.browseros.com'
const VIDEO_SRC = `${CDN_BASE_URL}/artifacts/claw/onboarding-recording/video.mp4`
const POSTER_SRC = `${CDN_BASE_URL}/artifacts/claw/onboarding-video/v0.2.0/first-run-demo-poster.png`

export function FirstRunVideo() {
  const reducedMotion = usePrefersReducedMotion()
  const ref = useRef<HTMLVideoElement>(null)
  useEffect(() => {
    const el = ref.current
    if (!el) return
    if (reducedMotion) {
      el.pause()
      return
    }
    // Muted + autoplay is allowed everywhere, but tab throttling or
    // an unlucky race between mount and the first-byte of the video
    // stream can leave the element paused. Kick play() explicitly
    // to close the gap.
    void el.play().catch(() => {
      // Blocked or errored; the poster stays visible until the
      // reader interacts. Extremely rare in practice.
    })
  }, [reducedMotion])
  return (
    <video
      ref={ref}
      src={VIDEO_SRC}
      poster={POSTER_SRC}
      preload="auto"
      autoPlay={!reducedMotion}
      muted
      loop={!reducedMotion}
      playsInline
      controls={reducedMotion}
      disablePictureInPicture
      aria-label="A short motion demo showing how BrowserClaw works: install the MCP, prompt your agent, watch the run land in this cockpit."
      className={
        reducedMotion
          ? 'aspect-video w-full select-none overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken object-contain'
          : 'pointer-events-none aspect-video w-full select-none overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken object-contain'
      }
    />
  )
}

function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(readPrefersReducedMotion)
  useEffect(() => {
    if (typeof window === 'undefined') {
      return
    }
    const mql = reducedMotionQuery()
    if (!mql) return
    const update = () => setReduced(mql.matches)
    update()
    mql.addEventListener('change', update)
    return () => mql.removeEventListener('change', update)
  }, [])
  return reduced
}

function readPrefersReducedMotion(): boolean {
  return reducedMotionQuery()?.matches ?? false
}

function reducedMotionQuery(): MediaQueryList | null {
  if (
    typeof window === 'undefined' ||
    typeof window.matchMedia !== 'function'
  ) {
    return null
  }
  return window.matchMedia('(prefers-reduced-motion: reduce)')
}
