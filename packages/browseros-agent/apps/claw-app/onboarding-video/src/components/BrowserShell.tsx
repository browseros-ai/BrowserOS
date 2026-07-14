/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The Chromium-style browser chrome wrapping the cockpit inside every
 * demo scene. Traffic lights, one active "BrowserClaw" tab, a
 * back/forward/reload toolbar, and a URL bar showing the standard
 * "Search Google or type a URL" placeholder (because the extension
 * ships as the browser's new-tab page).
 *
 * Kept composition-local, not extracted into the extension: the
 * running extension IS the tab, so the chrome around it never
 * renders inside the app. Only the video needs it to remind the
 * reader they are looking at a browser surface.
 *
 * The chrome height (BROWSER_CHROME_HEIGHT) is exported so scenes
 * that overlay callouts against sidebar positions can offset them
 * accurately.
 */

import type { CSSProperties, ReactNode } from 'react'
import { palette } from '../palette'

interface BrowserShellProps {
  children: ReactNode
  style?: CSSProperties
}

const RADIUS = 24
const TAB_STRIP_HEIGHT = 40
const TOOLBAR_HEIGHT = 46
const TAB_STRIP_BG = '#e7ebf1'

/** Total height (px) of the browser chrome above the app surface. */
export const BROWSER_CHROME_HEIGHT = TAB_STRIP_HEIGHT + TOOLBAR_HEIGHT + 1

export function BrowserShell({ children, style }: BrowserShellProps) {
  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        width: '100%',
        height: '100%',
        borderRadius: RADIUS,
        overflow: 'hidden',
        boxShadow: '0 40px 80px -20px rgba(10, 13, 20, 0.35)',
        border: `1px solid ${palette.border2}`,
        background: palette.bgCanvas,
        ...style,
      }}
    >
      <BrowserChrome />
      <div style={{ flex: 1, minHeight: 0 }}>{children}</div>
    </div>
  )
}

function BrowserChrome() {
  return (
    <div>
      <div
        style={{
          height: TAB_STRIP_HEIGHT,
          background: TAB_STRIP_BG,
          display: 'flex',
          alignItems: 'flex-end',
          padding: '0 14px',
        }}
      >
        <div
          style={{
            display: 'flex',
            gap: 8,
            alignSelf: 'center',
            marginRight: 16,
          }}
        >
          <TrafficLight color="#ff5f56" />
          <TrafficLight color="#ffbd2e" />
          <TrafficLight color="#27c93f" />
        </div>
        <div
          style={{
            height: 32,
            minWidth: 190,
            padding: '0 10px 0 12px',
            borderRadius: '10px 10px 0 0',
            background: palette.card,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
          }}
        >
          <div
            style={{
              width: 16,
              height: 16,
              borderRadius: 4,
              background: palette.accent,
              color: palette.card,
              fontSize: 10,
              fontWeight: 800,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
            }}
          >
            B
          </div>
          <span
            style={{
              flex: 1,
              fontSize: 12,
              fontWeight: 600,
              color: palette.ink,
              letterSpacing: -0.1,
            }}
          >
            BrowserClaw
          </span>
          <Glyph d="M5 5l6 6M11 5l-6 6" color={palette.ink3} size={13} />
        </div>
        <div style={{ alignSelf: 'center', padding: '0 10px' }}>
          <Glyph d="M8 3.5v9M3.5 8h9" color={palette.ink3} size={14} />
        </div>
      </div>
      <div
        style={{
          height: TOOLBAR_HEIGHT,
          background: palette.card,
          borderBottom: `1px solid ${palette.border2}`,
          display: 'flex',
          alignItems: 'center',
          gap: 14,
          padding: '0 16px',
        }}
      >
        <Glyph d="M10 3.5 5.5 8l4.5 4.5" color={palette.ink2} />
        <Glyph d="M6 3.5 10.5 8 6 12.5" color={palette.border2} />
        <Glyph
          d="M13 8a5 5 0 1 1-1.46-3.54M13 3.5V6h-2.5"
          color={palette.ink2}
        />
        <div
          style={{
            flex: 1,
            height: 30,
            borderRadius: 999,
            background: palette.bgSunken,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '0 14px',
          }}
        >
          <Glyph
            d="M11 7a4 4 0 1 1-8 0 4 4 0 0 1 8 0M9.9 9.9l2.6 2.6"
            color={palette.ink3}
            size={13}
          />
          <span style={{ fontSize: 12, color: palette.ink3 }}>
            Search Google or type a URL
          </span>
        </div>
        <div
          style={{
            display: 'flex',
            flexDirection: 'column',
            gap: 2.5,
            padding: '0 2px',
          }}
        >
          {['a', 'b', 'c'].map((k) => (
            <span
              key={k}
              style={{
                width: 3,
                height: 3,
                borderRadius: 999,
                background: palette.ink3,
              }}
            />
          ))}
        </div>
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

function Glyph({
  d,
  color,
  size = 16,
}: {
  d: string
  color: string
  size?: number
}) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" role="presentation">
      <path
        d={d}
        fill="none"
        stroke={color}
        strokeWidth={1.7}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  )
}
