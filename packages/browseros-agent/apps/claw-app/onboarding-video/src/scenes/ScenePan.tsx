/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Scene 03: the pan. The real `<CockpitOnboarding />` slides to
 * the left half; the agent terminal slides in from the right.
 * Labels above each surface name their role: "you watch here"
 * over the cockpit, "then: prompt your agent" over the terminal.
 * `AgentTerminal` stays composition-local because the extension
 * has no equivalent widget.
 */

import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from 'remotion'
import { CockpitOnboarding } from '@/components/cockpit/CockpitOnboarding'
import { AgentTerminal } from '../components/AgentTerminal'
import { BrowserShell } from '../components/BrowserShell'
import { SceneLabel } from '../components/SceneLabel'
import { palette } from '../palette'

const EASE_OUT = Easing.bezier(0.16, 1, 0.3, 1)

export function ScenePan() {
  const frame = useCurrentFrame()
  const cockpitX = interpolate(frame, [0, 50], [0, -400], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  const terminalX = interpolate(frame, [0, 50], [1200, 0], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  const cockpitScale = interpolate(frame, [0, 50], [1, 0.78], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  const terminalOpacity = interpolate(frame, [10, 55], [0, 1], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  const labelIn = interpolate(frame, [55, 80], [0, 1], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
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
