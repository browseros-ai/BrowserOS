/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Composition-only replacement for
 * `apps/claw-app/components/cockpit/FirstRunVideo.tsx`.
 *
 * The shipped component renders a native `<video autoplay loop>`
 * pointing at cdn.browseros.com. Two reasons this cannot be used
 * verbatim inside the Remotion composition:
 *
 *   1. Remotion's headless Chromium snapshots each frame instantly.
 *      The remote MP4 fetch never completes before capture, so the
 *      element renders as an empty background instead of the demo
 *      footage.
 *
 *   2. Even if we swapped in the poster PNG, that PNG is a still of
 *      this exact composition. Rendering it inside the composition
 *      produces a cockpit-inside-a-cockpit recursion that reads as
 *      broken UI, not a demo.
 *
 * The stub replaces the video widget with a neutral aspect-video
 * placeholder that reads as "media element sits here" without
 * mirroring the composition back at itself. Uses the same
 * design-token classes as the shipped component so the surrounding
 * layout stays honest.
 *
 * The swap is wired at the webpack layer in `remotion.config.ts`.
 * Extension builds are not affected because WXT + Vite has its
 * own resolver.
 */

export function FirstRunVideo() {
  return (
    <div
      role="img"
      aria-label="First-run motion demo (placeholder)"
      className="flex aspect-video w-full items-center justify-center overflow-hidden rounded-2xl border border-border-2 bg-bg-sunken"
    >
      <div className="flex items-center gap-3 font-mono text-[11px] text-ink-3 uppercase tracking-[0.16em]">
        <PlayGlyph />
        <span>Motion demo</span>
      </div>
    </div>
  )
}

function PlayGlyph() {
  return (
    <svg
      width="18"
      height="18"
      viewBox="0 0 24 24"
      fill="currentColor"
      role="presentation"
      aria-hidden="true"
    >
      <path d="M8 5v14l11-7L8 5z" />
    </svg>
  )
}
