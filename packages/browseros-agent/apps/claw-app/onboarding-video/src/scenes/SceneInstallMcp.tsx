/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Scene 02: the MCP install beat. Renders the real
 * `<CockpitOnboarding state="first-run" />` inside the browser
 * shell; the natural "Set up MCP endpoint" primary CTA already
 * carries the "install the MCP" message. A caption above the
 * shell names the beat.
 *
 * Animation: subtle scale settle + opacity fade at the wrapper.
 * The pulsing primary CTA is styled via the app's design tokens,
 * not a CSS keyframe, so it renders as a static Signal-blue
 * button — visually distinct enough without extra motion.
 */

import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from 'remotion'
import { CockpitOnboarding } from '@/components/cockpit/CockpitOnboarding'
import { BrowserShell } from '../components/BrowserShell'
import { SceneLabel } from '../components/SceneLabel'
import { palette } from '../palette'

const EASE_OUT = Easing.bezier(0.16, 1, 0.3, 1)

export function SceneInstallMcp() {
  const frame = useCurrentFrame()
  const labelIn = interpolate(frame, [0, 15], [0, 1], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  const shellScale = interpolate(frame, [0, 30], [0.99, 1], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  const shellOpacity = interpolate(frame, [0, 20], [0.4, 1], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  return (
    <AbsoluteFill style={{ background: palette.bgCanvas, padding: 24 }}>
      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 20,
          height: '100%',
        }}
      >
        <SceneLabel text="first: install the mcp" opacity={labelIn} />
        <div
          style={{
            flex: 1,
            minHeight: 0,
            scale: shellScale,
            opacity: shellOpacity,
            transformOrigin: 'top center',
          }}
        >
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
    </AbsoluteFill>
  )
}
