/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Scene 02: the MCP install beat. Camera stays on the cockpit; the
 * sidebar's `MCP` nav item lights up (via `CockpitFrame`'s
 * `highlightNav="MCP"` prop, which positions the highlight box
 * inside the sidebar's own flex layout so it cannot drift), a
 * floating endpoint card pops in with the local endpoint URL and
 * a Connect action, then flips to a "connected" green check.
 * Establishes the FIRST action the reader must take before their
 * agent can do anything: install BrowserClaw as an MCP.
 */

import { AbsoluteFill, Easing, interpolate, useCurrentFrame } from 'remotion'
import { BROWSER_CHROME_HEIGHT, CockpitFrame } from '../components/CockpitFrame'
import { SceneLabel } from '../components/SceneLabel'
import { fonts } from '../fonts'
import { palette } from '../palette'

const EASE_OUT = Easing.bezier(0.16, 1, 0.3, 1)

export function SceneInstallMcp() {
  const frame = useCurrentFrame()
  const labelIn = interpolate(frame, [0, 15], [0, 1], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  const highlightIn = interpolate(frame, [10, 30], [0, 1], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  const cardIn = interpolate(frame, [25, 55], [0, 1], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  const cardSlide = interpolate(frame, [25, 55], [-24, 0], {
    extrapolateRight: 'clamp',
    easing: EASE_OUT,
  })
  const connectPressed = frame >= 66
  const checkIn = interpolate(frame, [72, 92], [0, 1], {
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
        <div style={{ flex: 1, position: 'relative' }}>
          <CockpitFrame
            showLandingDot
            highlightNav="MCP"
            highlightIntensity={highlightIn}
          />
          <EndpointCard
            opacity={cardIn}
            translateX={cardSlide}
            connectPressed={connectPressed}
            checkOpacity={checkIn}
          />
        </div>
      </div>
    </AbsoluteFill>
  )
}

function EndpointCard({
  opacity,
  translateX,
  connectPressed,
  checkOpacity,
}: {
  opacity: number
  translateX: number
  connectPressed: boolean
  checkOpacity: number
}) {
  return (
    <div
      style={{
        position: 'absolute',
        left: 250,
        top: 140 + BROWSER_CHROME_HEIGHT,
        width: 420,
        borderRadius: 16,
        background: palette.card,
        border: `1px solid ${palette.border2}`,
        boxShadow: '0 30px 60px -20px rgba(10, 13, 20, 0.35)',
        padding: '18px 20px',
        display: 'flex',
        flexDirection: 'column',
        gap: 12,
        opacity,
        translate: `${translateX}px 0`,
      }}
    >
      <div
        style={{
          fontFamily: fonts.mono,
          fontSize: 10,
          letterSpacing: 1.6,
          color: palette.ink3,
        }}
      >
        MCP ENDPOINT
      </div>
      <div
        style={{
          fontFamily: fonts.mono,
          fontSize: 13,
          color: palette.ink,
        }}
      >
        http://127.0.0.1:9200/mcp
      </div>
      <div
        style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 4 }}
      >
        <div
          style={{
            padding: '6px 12px',
            borderRadius: 8,
            background: connectPressed ? palette.accentTint : palette.accent,
            color: connectPressed ? palette.accent : palette.card,
            border: `1px solid ${palette.accent}`,
            fontSize: 12,
            fontWeight: 700,
            scale: connectPressed ? 0.96 : 1,
            transformOrigin: 'center',
          }}
        >
          Connect
        </div>
        <div
          aria-hidden
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 6,
            color: palette.green,
            fontSize: 12,
            fontWeight: 700,
            opacity: checkOpacity,
            translate: `${interpolate(checkOpacity, [0, 1], [-8, 0])}px 0`,
          }}
        >
          <span
            style={{
              width: 16,
              height: 16,
              borderRadius: 999,
              background: palette.green,
              color: palette.card,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              fontSize: 12,
              fontWeight: 900,
            }}
          >
            ✓
          </span>
          Endpoint installed.
        </div>
      </div>
    </div>
  )
}
