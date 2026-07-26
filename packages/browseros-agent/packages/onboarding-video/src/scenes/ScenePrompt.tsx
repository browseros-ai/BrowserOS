/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Scene 03: the prompt. The reader watches the operator type a real
 * BrowserClaw instruction into the terminal, character by character.
 * On enter, an MCP packet leaves the terminal and travels to the
 * cockpit's landing-dot area.
 */

import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from 'remotion'
import { AgentTerminal } from '../components/AgentTerminal'
import { CockpitFrame } from '../components/CockpitFrame'
import { ConnectorPacket } from '../components/ConnectorPacket'
import { SceneLabel } from '../components/SceneLabel'
import { palette } from '../palette'

const PROMPT =
  '> use browserclaw to book me the cheapest morning flight from SFO to NYC next Friday.'

// Typewriter timing: prompt appears character by character over ~90 frames (3s).
const TYPE_START = 0
const TYPE_END = 90
// Send happens on enter at frame 95; packet flies frames 95-135 (~1.3s).
const SEND_FRAME = 95
const PACKET_END = 135

export function ScenePrompt() {
  const frame = useCurrentFrame()
  const charCount = Math.floor(
    interpolate(frame, [TYPE_START, TYPE_END], [0, PROMPT.length], {
      extrapolateRight: 'clamp',
      easing: Easing.linear,
    }),
  )
  const typingLine = PROMPT.slice(0, charCount)
  const caretOn = frame < SEND_FRAME && Math.floor(frame / 12) % 2 === 0
  const packetProgress = interpolate(frame, [SEND_FRAME, PACKET_END], [0, 1], {
    extrapolateLeft: 'clamp',
    extrapolateRight: 'clamp',
    easing: Easing.bezier(0.34, 1.2, 0.64, 1),
  })
  const packetOpacity =
    frame < SEND_FRAME
      ? 0
      : interpolate(frame, [PACKET_END - 15, PACKET_END], [1, 0], {
          extrapolateLeft: 'clamp',
          extrapolateRight: 'clamp',
        })
  return (
    <AbsoluteFill style={{ background: palette.bgCanvas, padding: 24 }}>
      {/* Cockpit at left, scaled */}
      <div
        style={{
          position: 'absolute',
          left: -340,
          top: 90,
          width: 900,
          height: 640,
          scale: 0.78,
          transformOrigin: 'top left',
        }}
      >
        <SceneLabel text="you watch here" style={{ marginBottom: 14 }} />
        <div style={{ height: 'calc(100% - 34px)' }}>
          <CockpitFrame showLandingDot />
        </div>
      </div>
      {/* Terminal at right */}
      <div
        style={{
          position: 'absolute',
          right: 60,
          top: 90,
          width: 720,
          height: 500,
        }}
      >
        <SceneLabel
          text="then: prompt your agent"
          style={{ marginBottom: 14 }}
        />
        <div style={{ height: 'calc(100% - 34px)' }}>
          <AgentTerminal
            lines={['$ claude']}
            typingLine={typingLine}
            showCaret={caretOn}
          />
        </div>
      </div>
      {/* MCP arrow: straight line from terminal left edge to cockpit
       *  right edge, both at their vertical midpoints (composition
       *  space). Terminal at composition x=790..1516, cockpit
       *  (scaled 0.78) at composition x=-316..386. Both midpoints
       *  land around y=364. */}
      <ConnectorPacket
        progress={packetProgress}
        opacity={packetOpacity}
        from={{ x: 790, y: 364 }}
        to={{ x: 400, y: 364 }}
      />
    </AbsoluteFill>
  )
}
