/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Scene 02: the pan. The cockpit slides to the left; an agent
 * terminal card slides in from the right. Both surfaces settle into
 * a side-by-side composition. Labels above each surface name their
 * role: "you watch here" over the cockpit, "your work happens here"
 * over the terminal.
 */

import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from 'remotion'
import { AgentTerminal } from '../components/AgentTerminal'
import { CockpitFrame } from '../components/CockpitFrame'
import { SceneLabel } from '../components/SceneLabel'
import { palette } from '../palette'

export function ScenePan() {
  const frame = useCurrentFrame()
  // Cockpit slides from centered to left half. Terminal slides in from off-canvas right.
  const cockpitX = interpolate(frame, [0, 50], [0, -400], {
    extrapolateRight: 'clamp',
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  })
  const terminalX = interpolate(frame, [0, 50], [1200, 0], {
    extrapolateRight: 'clamp',
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  })
  const cockpitScale = interpolate(frame, [0, 50], [1, 0.78], {
    extrapolateRight: 'clamp',
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  })
  const terminalOpacity = interpolate(frame, [10, 55], [0, 1], {
    extrapolateRight: 'clamp',
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  })
  const labelIn = interpolate(frame, [55, 80], [0, 1], {
    extrapolateRight: 'clamp',
    easing: Easing.bezier(0.16, 1, 0.3, 1),
  })
  return (
    <AbsoluteFill style={{ background: palette.bgCanvas, padding: 24 }}>
      <div
        style={{
          position: 'absolute',
          left: 60,
          top: 90,
          width: 900,
          height: 640,
          translate: `${cockpitX}px 0px`,
          scale: cockpitScale,
          transformOrigin: 'top left',
        }}
      >
        <SceneLabel
          text="you watch here"
          opacity={labelIn}
          style={{ marginBottom: 14 }}
        />
        <div style={{ height: 'calc(100% - 34px)' }}>
          <CockpitFrame showLandingDot />
        </div>
      </div>
      <div
        style={{
          position: 'absolute',
          right: 60,
          top: 90,
          width: 720,
          height: 500,
          translate: `${terminalX}px 0px`,
          opacity: terminalOpacity,
        }}
      >
        <SceneLabel
          text="then: prompt your agent"
          opacity={labelIn}
          style={{ marginBottom: 14 }}
        />
        <div style={{ height: 'calc(100% - 34px)' }}>
          <AgentTerminal lines={['$ claude']} typingLine="> " showCaret />
        </div>
      </div>
    </AbsoluteFill>
  )
}
