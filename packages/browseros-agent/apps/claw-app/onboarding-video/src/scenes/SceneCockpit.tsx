/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Scene 01: the cockpit. Now renders the real
 * `<CockpitOnboarding state="first-run" />` from the shipped
 * extension surface, wrapped in the composition-local `BrowserShell`
 * chrome. Any visual change to the app's first-run onboarding block
 * automatically propagates to the intro video from now on.
 *
 * The scene doubles as the composition poster: frame 0 must render
 * fully opaque, so the entrance animation is a subtle scale settle
 * (0.99 -> 1.00) with no opacity fade.
 */

import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from 'remotion'
import { CockpitOnboarding } from '@/components/cockpit/CockpitOnboarding'
import { BrowserShell } from '../components/BrowserShell'
import { SceneLabel } from '../components/SceneLabel'
import { palette } from '../palette'

export function SceneCockpit() {
  const frame = useCurrentFrame()
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
        <div style={{ flex: 1, minHeight: 0 }}>
          <BrowserShell>
            {/* CockpitOnboarding scrolls inside the extension's
             *  newtab tab; in the video frame, we render it at
             *  natural size and crop to the browser-shell viewport.
             *  The above-the-fold portion (hero + motion-demo
             *  placeholder + start of the primary CTA row) is what
             *  a real reader sees on their first paint anyway. */}
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
