/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The MCP connector: a straight arrow from the agent terminal
 * (right) to the cockpit (left) with an "MCP" pill sitting on
 * the line. Reads as a plain "A -> B" flow so the reader instantly
 * understands the agent is calling the browser via MCP.
 * `progress` fades the arrow in during the packet-fly phase;
 * `opacity` handles the outer fade-out at the end of the scene.
 */

import { fonts } from '../fonts'
import { palette } from '../palette'

interface ConnectorPacketProps {
  /** 0 to 1: overall arrow fade-in during the packet phase. */
  progress: number
  /** Composition-space start (terminal side). */
  from: { x: number; y: number }
  /** Composition-space end (cockpit side). */
  to: { x: number; y: number }
  /** Overall element opacity for the outer fade-out. */
  opacity?: number
}

const STROKE_WIDTH = 8
const ARROWHEAD_ID = 'mcp-arrowhead'

export function ConnectorPacket({
  progress,
  from,
  to,
  opacity = 1,
}: ConnectorPacketProps) {
  // Fade the arrow in over the first 40% of `progress`, hold the
  // rest so the reader has plenty of time to parse the shape.
  const fadeIn = progress <= 0 ? 0 : progress >= 0.4 ? 1 : progress / 0.4
  const midX = (from.x + to.x) / 2
  const midY = (from.y + to.y) / 2
  return (
    <svg
      role="presentation"
      // NB. left/top offset by -24 to escape the parent AbsoluteFill's
      // padding: 24 so this SVG's viewBox aligns 1:1 with composition
      // space. Callers pass composition-space coordinates for `from`
      // and `to` without needing to subtract the padding.
      style={{
        position: 'absolute',
        left: -24,
        top: -24,
        width: 1600,
        height: 900,
        opacity: opacity * fadeIn,
        pointerEvents: 'none',
      }}
      viewBox="0 0 1600 900"
    >
      <defs>
        <marker
          id={ARROWHEAD_ID}
          viewBox="0 0 12 12"
          refX="10"
          refY="6"
          markerWidth="8"
          markerHeight="8"
          orient="auto-start-reverse"
          markerUnits="userSpaceOnUse"
        >
          <path d="M0 0 L12 6 L0 12 z" fill={palette.accent} />
        </marker>
      </defs>
      <line
        x1={from.x}
        y1={from.y}
        x2={to.x}
        y2={to.y}
        stroke={palette.accent}
        strokeWidth={STROKE_WIDTH}
        strokeLinecap="round"
        markerEnd={`url(#${ARROWHEAD_ID})`}
      />
      {/* MCP label pill centred on the line. */}
      <g transform={`translate(${midX - 52} ${midY - 20})`}>
        <rect
          width={104}
          height={38}
          rx={19}
          fill={palette.accent}
          stroke={palette.card}
          strokeWidth={3}
        />
        <text
          x={52}
          y={25}
          fontFamily={fonts.mono}
          fontSize={17}
          fontWeight={800}
          letterSpacing={3}
          textAnchor="middle"
          fill={palette.card}
        >
          MCP
        </text>
      </g>
    </svg>
  )
}
