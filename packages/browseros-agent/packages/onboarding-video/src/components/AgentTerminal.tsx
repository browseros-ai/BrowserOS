/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Stylised agent-terminal card used as the right-hand surface once
 * the camera pans away from the cockpit. Style mirrors the Terminal
 * component in claw-app CockpitOnboarding.tsx so the demo reads as
 * a real Claude Code / Cursor / Codex terminal.
 */

import type { CSSProperties } from 'react'
import { fonts } from '../fonts'
import { palette } from '../palette'

interface AgentTerminalProps {
  /** Lines already committed to the terminal buffer. */
  lines: readonly string[]
  /**
   * Optional in-progress line at the bottom, character-count
   * controlled by the caller (typewriter effect).
   */
  typingLine?: string
  /** When true, renders a solid caret at the end of the typing line. */
  showCaret?: boolean
  style?: CSSProperties
}

export function AgentTerminal({
  lines,
  typingLine,
  showCaret,
  style,
}: AgentTerminalProps) {
  return (
    <div
      style={{
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        borderRadius: 20,
        overflow: 'hidden',
        border: `1px solid ${palette.border2}`,
        background: '#0f1420',
        boxShadow: '0 40px 80px -20px rgba(10, 13, 20, 0.55)',
        ...style,
      }}
    >
      <div
        style={{
          padding: '10px 16px',
          background: '#161b2a',
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          borderBottom: '1px solid #232839',
        }}
      >
        <TrafficLight color="#ff5f56" />
        <TrafficLight color="#ffbd2e" />
        <TrafficLight color="#27c93f" />
        <div
          style={{
            marginLeft: 12,
            color: '#8a92a8',
            fontSize: 11,
            fontFamily: fonts.mono,
          }}
        >
          agent, claude-code
        </div>
      </div>
      <div
        style={{
          flex: 1,
          padding: '22px 24px',
          fontFamily: fonts.mono,
          fontSize: 14,
          lineHeight: 1.55,
          color: '#e0e4ee',
          display: 'flex',
          flexDirection: 'column',
          gap: 6,
        }}
      >
        {lines.map((line, i) => (
          <div
            // biome-ignore lint/suspicious/noArrayIndexKey: fixed set of pre-composed lines
            key={i}
            style={{ color: line.startsWith('>') ? '#e0e4ee' : '#8a92a8' }}
          >
            {line}
          </div>
        ))}
        {typingLine !== undefined && (
          <div style={{ color: '#e0e4ee', display: 'flex' }}>
            <span>{typingLine}</span>
            {showCaret && (
              <span
                style={{
                  display: 'inline-block',
                  width: 8,
                  height: 18,
                  marginLeft: 2,
                  background: palette.accent,
                  translate: '0 3px',
                }}
              />
            )}
          </div>
        )}
      </div>
    </div>
  )
}

function TrafficLight({ color }: { color: string }) {
  return (
    <span
      style={{
        display: 'inline-block',
        width: 10,
        height: 10,
        borderRadius: 999,
        background: color,
      }}
    />
  )
}
