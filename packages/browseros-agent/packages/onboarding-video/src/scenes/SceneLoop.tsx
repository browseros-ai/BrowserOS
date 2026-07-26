/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Scene 06: the CTA + loop reset. First half holds the "set it up
 * below" caption + a downward chevron so the reader learns the
 * scroll affordance BEFORE the loop restarts; second half fades
 * back to the initial cockpit + landing-dot state so Scene 01 can
 * restart without a visible seam.
 */

import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from 'remotion'
import { CockpitFrame } from '../components/CockpitFrame'
import { SceneLabel } from '../components/SceneLabel'
import { palette } from '../palette'

const EASE_OUT = Easing.bezier(0.16, 1, 0.3, 1)
const EASE_STANDARD = Easing.bezier(0.4, 0, 0.6, 1)

export function SceneLoop() {
  const frame = useCurrentFrame()
  // Fade in the cockpit chrome so the loop returns to the poster.
  const cockpitOpacity = interpolate(frame, [0, 30], [0, 1], {
    extrapolateRight: 'clamp',
    easing: EASE_STANDARD,
  })
  // CTA caption: appears for the first ~2s of the scene, then fades
  // out over the last ~1s. Chevron bounces subtly for the whole time.
  const ctaIn = interpolate(frame, [0, 12], [0, 1], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  const ctaOut = interpolate(frame, [60, 90], [1, 0], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
    easing: EASE_STANDARD,
  })
  const ctaOpacity = Math.min(ctaIn, ctaOut)
  // Bounce the chevron up-and-down every second (~30 frames).
  const bounceProgress = (frame % 30) / 30
  const chevronY = interpolate(bounceProgress, [0, 0.5, 1], [0, 8, 0], {
    easing: EASE_STANDARD,
  })
  return (
    <AbsoluteFill style={{ background: palette.bgCanvas, padding: 24 }}>
      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 20,
          height: '100%',
          opacity: cockpitOpacity,
        }}
      >
        <SceneLabel text="you are here" opacity={cockpitOpacity} />
        <div style={{ flex: 1 }}>
          <CockpitFrame showLandingDot />
        </div>
      </div>
      <div
        style={{
          position: 'absolute',
          left: 0,
          right: 0,
          bottom: 60,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          flexDirection: 'column',
          gap: 12,
          opacity: ctaOpacity,
        }}
      >
        <div
          style={{
            padding: '10px 22px',
            borderRadius: 999,
            background: palette.accent,
            color: palette.card,
            fontSize: 18,
            fontWeight: 700,
            letterSpacing: -0.2,
            boxShadow: '0 20px 40px -12px rgba(2, 84, 236, 0.55)',
          }}
        >
          Set it up below
        </div>
        <div
          aria-hidden
          style={{
            width: 44,
            height: 44,
            borderRadius: 999,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            background: palette.card,
            border: `1px solid ${palette.border2}`,
            color: palette.accent,
            fontWeight: 900,
            fontSize: 22,
            translate: `0 ${chevronY}px`,
            boxShadow: '0 10px 24px -10px rgba(10, 13, 20, 0.25)',
          }}
        >
          ↓
        </div>
      </div>
    </AbsoluteFill>
  )
}
