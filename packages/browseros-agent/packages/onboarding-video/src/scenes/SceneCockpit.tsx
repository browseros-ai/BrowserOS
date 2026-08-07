/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Scene 01: the cockpit. Fades in the BrowserClaw chrome and lands
 * the "first run will land here" dot marker with a small bounce. No
 * agent surface visible yet; the reader anchors on the destination.
 */

import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from 'remotion'
import { CockpitFrame } from '../components/CockpitFrame'
import { SceneLabel } from '../components/SceneLabel'
import { palette } from '../palette'

export function SceneCockpit() {
  const frame = useCurrentFrame()
  // Scene 01 doubles as the composition poster: the very first
  // frame (frame 0) needs to render fully opaque so a reader whose
  // browser blocks autoplay still sees a meaningful still frame.
  // Subtle scale settle only, no entrance fade.
  const scale = interpolate(frame, [0, 30], [0.99, 1], {
    extrapolateRight: 'clamp',
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  })
  const labelIn = interpolate(frame, [15, 40], [0.35, 1], {
    extrapolateRight: 'clamp',
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  })
  return (
    <AbsoluteFill style={{ background: palette.bgCanvas, padding: 24 }}>
      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 20,
          height: '100%',
          scale,
        }}
      >
        <SceneLabel text="you are here" opacity={labelIn} />
        <div style={{ flex: 1 }}>
          <CockpitFrame showLandingDot />
        </div>
      </div>
    </AbsoluteFill>
  )
}
