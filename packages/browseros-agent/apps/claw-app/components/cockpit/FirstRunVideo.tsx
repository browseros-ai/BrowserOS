/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Renders the ~20-second cockpit first-run motion demo. Ships as a
 * native `<video autoplay muted loop playsinline>` that streams from
 * versioned Cloudflare R2 objects through the BrowserOS CDN. Chromium
 * always allows muted autoplay without a user gesture. Reduced-motion
 * readers get the same video with autoplay and looping disabled, plus
 * native controls so playback starts from an explicit interaction.
 *
 * How the video URL got here
 *
 * The MP4 + poster are NOT tracked in git. The source composition is
 * versioned in `packages/browseros-agent/packages/onboarding-video/`;
 * rendered assets are uploaded to versioned R2 keys under
 * `artifacts/claw/onboarding-video/v<version>/` and served publicly from
 * `https://cdn.browseros.com`. That indirection keeps the extension
 * bundle small and the repo history clean.
 *
 * To bump the video:
 *
 *   1. Edit the composition source in
 *      `packages/browseros-agent/packages/onboarding-video/`.
 *
 *   2. Bump `packages/onboarding-video/package.json` to a new version.
 *      Never reuse an existing version: the upload script guards against
 *      overwriting R2 objects, and the versioned CDN URL is the cache
 *      buster clients see.
 *
 *   3. Render locally:
 *        cd packages/browseros-agent
 *        bun run --cwd packages/onboarding-video render
 *        bun run --cwd packages/onboarding-video render:poster
 *
 *   4. Upload the rendered MP4 + poster to R2:
 *        bun run upload:onboarding-video
 *
 *      The script reads R2 credentials from process env or
 *      `apps/server/.env.production`, writes keys below
 *      `artifacts/claw/onboarding-video/v<version>/`, and prints the
 *      public CDN URLs. Use `--force` only for an intentional overwrite.
 *
 *   5. Update `ASSET_VERSION` below to the uploaded package version.
 *
 * Because clients request versioned CDN URLs, a URL bump should come from
 * a package-version bump plus fresh R2 objects. Overwriting an existing
 * version can leave clients on the cached old asset.
 */

import { useEffect, useRef, useState } from 'react'

const CDN_BASE_URL = 'https://cdn.browseros.com'
const ASSET_VERSION = '0.2.0'
const ASSET_BASE = `${CDN_BASE_URL}/artifacts/claw/onboarding-video/v${ASSET_VERSION}`
const VIDEO_SRC = `${ASSET_BASE}/first-run-demo.mp4`
const POSTER_SRC = `${ASSET_BASE}/first-run-demo-poster.png`

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
