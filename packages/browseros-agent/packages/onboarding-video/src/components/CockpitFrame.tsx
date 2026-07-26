/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The BrowserClaw cockpit chrome as it appears inside the demo. A
 * stylised recreation of the actual app: sidebar wordmark, header
 * strip, empty recent-activity table, one live-task row optional.
 * Wrapped in a minimal Chromium-style browser shell (traffic lights,
 * tab, URL bar) so readers see the cockpit lives inside their
 * browser. The URL bar shows the new-tab placeholder because
 * claw-app ships as the browser's new-tab page.
 * Kept SVG-simple so the render stays under budget and the visual
 * matches the app without a screenshot dependency.
 */

import type { CSSProperties } from 'react'
import { fonts } from '../fonts'
import { palette } from '../palette'

interface CockpitFrameProps {
  /** Optional live-task row rendered in the recent-activity table. */
  liveTask?: {
    agent: string
    action: string
    /** 0 to 1 progress. */
    progress: number
  }
  /** When true, renders a pulsing "first run will land here" dot marker in the table. */
  showLandingDot?: boolean
  /**
   * When set, wraps the named sidebar nav item in an accent-tinted
   * highlight box. Positioned inside the sidebar's flex layout so
   * the highlight cannot drift from the item across renders.
   */
  highlightNav?: 'Cockpit' | 'MCP' | 'Audit' | 'Agents'
  /** 0 to 1, scales the highlight box for a pop-in animation. */
  highlightIntensity?: number
  style?: CSSProperties
}

const RADIUS = 24
const TAB_STRIP_HEIGHT = 40
const TOOLBAR_HEIGHT = 46
const TAB_STRIP_BG = '#e7ebf1'

/**
 * Height the browser shell adds above the app surface. Scenes that
 * pin overlays against sidebar positions offset by this.
 */
export const BROWSER_CHROME_HEIGHT = TAB_STRIP_HEIGHT + TOOLBAR_HEIGHT + 1

export function CockpitFrame({
  liveTask,
  showLandingDot,
  highlightNav,
  highlightIntensity = 1,
  style,
}: CockpitFrameProps) {
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
      <div style={{ flex: 1, minHeight: 0, display: 'flex' }}>
        <Sidebar
          highlightNav={highlightNav}
          highlightIntensity={highlightIntensity}
        />
        <MainColumn liveTask={liveTask} showLandingDot={showLandingDot} />
      </div>
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

function Sidebar({
  highlightNav,
  highlightIntensity,
}: {
  highlightNav?: CockpitFrameProps['highlightNav']
  highlightIntensity: number
}) {
  return (
    <div
      style={{
        width: 210,
        background: palette.bgSunken,
        padding: '32px 22px',
        display: 'flex',
        flexDirection: 'column',
        gap: 22,
        borderRight: `1px solid ${palette.border2}`,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <div
          style={{
            width: 32,
            height: 32,
            borderRadius: 9,
            background: palette.accent,
            color: palette.card,
            fontWeight: 800,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          B
        </div>
        <div
          style={{
            fontWeight: 800,
            fontSize: 15,
            letterSpacing: -0.3,
            color: palette.ink,
          }}
        >
          BrowserClaw
        </div>
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <SidebarItem
          label="Cockpit"
          active
          highlighted={highlightNav === 'Cockpit'}
          highlightIntensity={highlightIntensity}
        />
        <SidebarItem
          label="MCP"
          highlighted={highlightNav === 'MCP'}
          highlightIntensity={highlightIntensity}
        />
        <SidebarItem
          label="Audit"
          highlighted={highlightNav === 'Audit'}
          highlightIntensity={highlightIntensity}
        />
        <SidebarItem
          label="Agents"
          highlighted={highlightNav === 'Agents'}
          highlightIntensity={highlightIntensity}
        />
      </div>
    </div>
  )
}

function SidebarItem({
  label,
  active,
  highlighted,
  highlightIntensity,
}: {
  label: string
  active?: boolean
  highlighted?: boolean
  highlightIntensity: number
}) {
  return (
    <div
      style={{
        position: 'relative',
        padding: '8px 12px',
        borderRadius: 8,
        fontSize: 13,
        color: active ? palette.ink : palette.ink3,
        background: active ? palette.card : 'transparent',
        border: active
          ? `1px solid ${palette.border2}`
          : '1px solid transparent',
        fontWeight: active ? 600 : 500,
      }}
    >
      {highlighted && (
        <div
          aria-hidden
          style={{
            position: 'absolute',
            inset: 0,
            borderRadius: 8,
            border: `2px solid ${palette.accent}`,
            boxShadow: `0 0 0 6px ${palette.accentTint}`,
            opacity: highlightIntensity,
            scale: 0.95 + highlightIntensity * 0.05,
            pointerEvents: 'none',
          }}
        />
      )}
      {label}
    </div>
  )
}

function MainColumn({
  liveTask,
  showLandingDot,
}: {
  liveTask?: CockpitFrameProps['liveTask']
  showLandingDot?: boolean
}) {
  return (
    <div
      style={{
        flex: 1,
        padding: '38px 44px',
        display: 'flex',
        flexDirection: 'column',
        gap: 28,
      }}
    >
      <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
        <div
          style={{
            fontSize: 10,
            color: palette.ink3,
            letterSpacing: 2,
            fontFamily: fonts.mono,
          }}
        >
          COCKPIT
        </div>
        <div
          style={{
            fontSize: 30,
            fontWeight: 800,
            color: palette.ink,
            letterSpacing: -0.6,
          }}
        >
          Recent activity
        </div>
      </div>
      <div
        style={{
          flex: 1,
          borderRadius: 16,
          border: `1px solid ${palette.border2}`,
          background: palette.card,
          padding: 20,
          display: 'flex',
          flexDirection: 'column',
          gap: 12,
        }}
      >
        <TableHeader />
        {liveTask ? (
          <LiveRow task={liveTask} />
        ) : showLandingDot ? (
          <LandingDotRow />
        ) : (
          <EmptyRow />
        )}
      </div>
    </div>
  )
}

function TableHeader() {
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: '1fr 2fr 1fr 1fr',
        fontSize: 10,
        color: palette.ink3,
        letterSpacing: 1.6,
        fontFamily: fonts.mono,
        borderBottom: `1px solid ${palette.border2}`,
        paddingBottom: 8,
      }}
    >
      <span>AGENT</span>
      <span>ACTION</span>
      <span>STATUS</span>
      <span style={{ textAlign: 'right' }}>WHEN</span>
    </div>
  )
}

function EmptyRow() {
  return (
    <div
      style={{
        color: palette.ink3,
        fontSize: 12,
        padding: 22,
        textAlign: 'center',
      }}
    >
      Waiting for your first run.
    </div>
  )
}

function LandingDotRow() {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '18px 8px',
      }}
    >
      <div
        style={{
          width: 10,
          height: 10,
          borderRadius: 999,
          background: palette.accent,
          boxShadow: `0 0 0 6px ${palette.accentTint}`,
        }}
      />
      <div style={{ fontSize: 12, color: palette.ink2, fontStyle: 'italic' }}>
        First run will land here.
      </div>
    </div>
  )
}

function LiveRow({
  task,
}: {
  task: NonNullable<CockpitFrameProps['liveTask']>
}) {
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: '1fr 2fr 1fr 1fr',
        alignItems: 'center',
        fontSize: 13,
        padding: '10px 0',
      }}
    >
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          color: palette.ink,
          fontWeight: 600,
        }}
      >
        <div
          style={{
            width: 8,
            height: 8,
            borderRadius: 999,
            background: palette.accent,
          }}
        />
        {task.agent}
      </div>
      <div style={{ color: palette.ink2 }}>{task.action}</div>
      <div>
        <div
          style={{
            width: 120,
            height: 6,
            background: palette.bgSunken,
            borderRadius: 999,
            overflow: 'hidden',
          }}
        >
          <div
            style={{
              width: `${Math.round(task.progress * 100)}%`,
              height: '100%',
              background: palette.accent,
            }}
          />
        </div>
      </div>
      <div style={{ textAlign: 'right', color: palette.ink3 }}>just now</div>
    </div>
  )
}
