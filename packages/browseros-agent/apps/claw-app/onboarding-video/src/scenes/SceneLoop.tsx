/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Scene 06: the CTA + loop reset. Real `<CockpitOnboarding />`
 * fades back in behind a "set it up below" pill + bouncing
 * downward chevron so the reader learns the scroll affordance
 * before the loop restarts. Second half fades the CTA out so
 * scene 01 can restart without a visible seam.
 */

import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from 'remotion'
import { CockpitOnboarding } from '@/components/cockpit/CockpitOnboarding'
import { BrowserShell } from '../components/BrowserShell'
import { SceneLabel } from '../components/SceneLabel'
import { palette } from '../palette'

const EASE_OUT = Easing.bezier(0.16, 1, 0.3, 1)
const EASE_STANDARD = Easing.bezier(0.4, 0, 0.6, 1)

export function SceneLoop() {
  const frame = useCurrentFrame()
  const cockpitOpacity = interpolate(frame, [0, 30], [0, 1], {
    extrapolateRight: 'clamp',
    easing: EASE_STANDARD,
  })
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
        <div style={{ flex: 1, minHeight: 0 }}>
          <BrowserShell>
            <div
              style={{
                height: '100%',
                overflow: 'hidden',
                background: 'var(--color-bg-canvas)',
              }}
            >
              <CockpitOnboarding state="first-run" />
            </div>
          </BrowserShell>
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
