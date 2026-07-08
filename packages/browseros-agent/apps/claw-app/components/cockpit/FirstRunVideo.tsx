/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Renders the ~20-second cockpit first-run motion demo. Ships as a
 * native `<video autoplay muted loop playsinline>` that streams
 * from a versioned GitHub Release asset. Chromium always allows
 * muted autoplay without a user gesture. Reduced-motion readers see
 * the poster PNG (rendered from frame 0 of the composition).
 *
 * ─── how the video URL got here ────────────────────────────────
 *
 * The MP4 + poster are NOT tracked in git. They live as release
 * artefacts of the source composition, which is versioned in
 * `packages/browseros-agent/packages/onboarding-video/`. That
 * indirection keeps the extension bundle small (~250 KB gz saved)
 * and the repo history clean (no blob per render).
 *
 * To bump the video:
 *
 *   1. Edit the composition source in `packages/onboarding-video/`.
 *
 *   2. Render locally:
 *        cd packages/browseros-agent/packages/onboarding-video
 *        bun run render          # writes out/first-run-demo.mp4
 *        bun run render:poster   # writes out/first-run-demo-poster.png
 *
 *   3. Publish as a new GitHub Release. Never reuse an existing tag
 *      (Chromium caches these URLs aggressively; a new tag is the
 *      only reliable cache-buster):
 *        VERSION=v0.2.0
 *        gh release create onboarding-video/$VERSION \
 *          --repo browseros-ai/BrowserOS \
 *          --target <branch-that-has-the-source> \
 *          --prerelease \
 *          --title "Onboarding video $VERSION (preview)" \
 *          --notes "One-line summary of what changed." \
 *          packages/browseros-agent/packages/onboarding-video/out/first-run-demo.mp4 \
 *          packages/browseros-agent/packages/onboarding-video/out/first-run-demo-poster.png
 *
 *   4. Update the two `RELEASE_*` constants below to point at the
 *      new tag and commit.
 *
 * The GitHub Releases CDN sets
 *   `Cache-Control: public, max-age=31536000, immutable`
 * on release-download URLs, so the browser fetches each URL exactly
 * once per client and reuses the local copy for a year. That is
 * why a URL bump requires cutting a new tag, not overwriting the
 * existing asset in place. Overwriting leaves clients staring at
 * the cached old version until they hard-reload.
 *
 * ─── why not just curl on setup ────────────────────────────────
 *
 * The alternative pattern of `bun run video:fetch` pulling into
 * `public/onboarding/` at setup time was tried and dropped. Fetching
 * at runtime removes an extra build step, keeps the repo bundle
 * lean, and the browser cache does the heavy lifting once the first
 * visitor loads it.
 */

import { useEffect, useRef, useState } from 'react'

const RELEASE_TAG = 'onboarding-video/v0.1.0'
const RELEASE_BASE = `https://github.com/browseros-ai/BrowserOS/releases/download/${RELEASE_TAG}`
const VIDEO_SRC = `${RELEASE_BASE}/first-run-demo.mp4`
const POSTER_SRC = `${RELEASE_BASE}/first-run-demo-poster.png`

export function FirstRunVideo() {
  const reducedMotion = usePrefersReducedMotion()
  const ref = useRef<HTMLVideoElement>(null)
  useEffect(() => {
    if (reducedMotion) return
    // Muted + autoplay is allowed everywhere, but tab throttling or
    // an unlucky race between mount and the first-byte of the video
    // stream can leave the element paused. Kick play() explicitly
    // to close the gap.
    const el = ref.current
    if (!el) return
    void el.play().catch(() => {
      // Blocked or errored; the poster stays visible until the
      // reader interacts. Extremely rare in practice.
    })
  }, [reducedMotion])
  if (reducedMotion) {
    return (
      <img
        src={POSTER_SRC}
        alt=""
        aria-hidden
        className="aspect-video w-full select-none overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken object-contain"
      />
    )
  }
  return (
    <video
      ref={ref}
      src={VIDEO_SRC}
      poster={POSTER_SRC}
      preload="auto"
      autoPlay
      muted
      loop
      playsInline
      controls={false}
      disablePictureInPicture
      aria-label="A short motion demo showing how BrowserClaw works: install the MCP, prompt your agent, watch the run land in this cockpit."
      className="pointer-events-none aspect-video w-full select-none overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken object-contain"
    />
  )
}

function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(false)
  useEffect(() => {
    if (
      typeof window === 'undefined' ||
      typeof window.matchMedia !== 'function'
    ) {
      return
    }
    const mql = window.matchMedia('(prefers-reduced-motion: reduce)')
    const update = () => setReduced(mql.matches)
    update()
    mql.addEventListener('change', update)
    return () => mql.removeEventListener('change', update)
  }, [])
  return reduced
}
