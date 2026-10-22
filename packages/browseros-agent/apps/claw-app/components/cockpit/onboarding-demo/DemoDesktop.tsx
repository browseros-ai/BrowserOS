/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Folder } from 'lucide-react'
import type { ReactNode } from 'react'
import { ClaudeAiIcon } from '@/components/ui/svgs/claudeAiIcon'
import { CodexDark } from '@/components/ui/svgs/codexDark'
import { cn } from '@/lib/utils'
import { DemoClaudeWindow } from './DemoClaudeWindow'
import { DemoNeoWindow } from './DemoNeoWindow'
import { DemoNeoMark } from './demo-parts'
import type { DemoFrame } from './timeline'

/** macOS draws the dock's hover label in the system face, not the app's. */
const MAC_FONT =
  "font-[-apple-system,BlinkMacSystemFont,'SF_Pro_Text','Helvetica_Neue',sans-serif]"

/**
 * The 960×480 drawn desktop at one frame of the timeline: the two windows on
 * the player's plain ground, with the dock. No menu bar or wallpaper (owner):
 * the windows take that space instead.
 */
export function DemoDesktop({
  frame,
  spinGlyph,
}: {
  frame: DemoFrame
  spinGlyph: string
}) {
  const claudeFront = frame.front === 'claude'
  return (
    <>
      <DemoNeoWindow frame={frame.neo} fading={frame.fading} />
      <DemoClaudeWindow
        frame={frame.claude}
        front={claudeFront}
        fading={frame.fading}
        spinGlyph={spinGlyph}
      />
      <Dock frame={frame} />
      <Pointer pointer={frame.pointer} />
      <AgentPointer at={frame.neo.agentPointer} />
    </>
  )
}

function DockApp({
  children,
  running,
  bounce,
  tip,
}: {
  children: ReactNode
  running?: boolean
  bounce?: boolean
  tip?: { label: string; on: boolean }
}) {
  return (
    <div className="relative flex flex-col items-center gap-[5px]">
      <span className={cn('block', bounce && 'animate-demo-bounce')}>
        {children}
      </span>
      <i
        className={cn(
          'size-1 rounded-full',
          running ? 'bg-ink-deep' : 'bg-transparent',
        )}
      />
      {tip && (
        <span
          className={cn(
            'pointer-events-none absolute bottom-[calc(100%+10px)] left-1/2 -translate-x-1/2 whitespace-nowrap rounded-[7px] border border-white/20 bg-[rgb(29_31_36/0.88)] px-2.5 py-1 font-medium text-[12px] text-white leading-4 transition-opacity duration-200',
            MAC_FONT,
            tip.on ? 'opacity-100' : 'opacity-0',
          )}
        >
          {tip.label}
        </span>
      )}
    </div>
  )
}

const TILE =
  'grid size-11 place-items-center overflow-hidden rounded-[11px] border'

/** Finder, Terminal, Claude, Codex, neo: the Claude and Codex tiles use the
 * marks the product already ships, so people recognise their own dock. */
function Dock({ frame }: { frame: DemoFrame }) {
  return (
    <div className="absolute top-[404px] left-1/2 z-6 flex h-16 -translate-x-1/2 items-start gap-2.5 rounded-[20px] border border-white/95 bg-card/60 px-[11px] pt-2 shadow-[0_24px_44px_-18px_rgb(1_18_63/0.38),0_2px_8px_rgb(1_18_63/0.08)] backdrop-blur-[10px] dark:border-white/10">
      <DockApp>
        <span className={cn(TILE, 'border-border-2 bg-card text-accent-2')}>
          <Folder
            aria-hidden
            className="size-6 fill-accent-tint"
            strokeWidth={1.8}
          />
        </span>
      </DockApp>
      <DockApp>
        <span className={cn(TILE, 'border-border bg-muted')}>
          <span className="grid h-7 w-[34px] place-items-center rounded-[5px] bg-ink-deep font-bold font-mono text-[13px] text-white">
            &gt;_
          </span>
        </span>
      </DockApp>
      <DockApp
        running={frame.dock.claudeRunning}
        bounce={frame.dock.claudeBounce}
        tip={{ label: 'Claude', on: frame.dock.tip }}
      >
        <span className={cn(TILE, 'border-[#C9694A] bg-[#D97757]')}>
          <ClaudeAiIcon
            aria-hidden
            className="size-[27px] [&_path]:fill-white"
          />
        </span>
      </DockApp>
      <DockApp>
        <span className={cn(TILE, 'border-[#15171C] bg-[#15171C]')}>
          <CodexDark aria-hidden className="size-[30px]" />
        </span>
      </DockApp>
      <DockApp running>
        <DemoNeoMark className="size-11 rounded-[11px] shadow-[0_10px_18px_-8px_rgb(2_84_236/0.6)]" />
      </DockApp>
    </div>
  )
}

const ARROW_PATH = 'M1.5 1.5v19.2l4.6-4.4 3 7.1 3.2-1.4-3-6.9h6.3z'

/** Your pointer: crosses the desktop to the Claude icon and opens it. */
function Pointer({ pointer }: { pointer: DemoFrame['pointer'] }) {
  return (
    <svg
      aria-hidden
      viewBox="0 0 18 26"
      className={cn(
        'absolute z-9 h-[26px] w-[18px] transition-[left,top,opacity,transform] duration-[1100ms,1100ms,300ms,120ms] ease-[cubic-bezier(.3,.1,.2,1)]',
        pointer.atDock
          ? 'top-[428px] left-[474px]'
          : 'top-[260px] left-[640px]',
        pointer.on ? 'opacity-100' : 'opacity-0',
        pointer.pressed && 'scale-[0.88]',
      )}
    >
      <path
        d={ARROW_PATH}
        className="fill-ink-deep stroke-white"
        strokeWidth="1.5"
        strokeLinejoin="round"
      />
    </svg>
  )
}

const AGENT_SPOTS = {
  off: 'top-[194px] left-[760px]',
  to: 'top-[130px] left-[738px]',
  when: 'top-[130px] left-[872px]',
  answer: 'top-[274px] left-[784px]',
} as const

/** The agent's pointer, tagged "claude", doing the work inside neo. */
function AgentPointer({ at }: { at: DemoFrame['neo']['agentPointer'] }) {
  return (
    <div
      aria-hidden
      className={cn(
        'absolute z-9 flex flex-col items-start transition-[left,top,opacity] duration-500 ease-out',
        AGENT_SPOTS[at],
        at === 'off' ? 'opacity-0' : 'opacity-100',
      )}
    >
      <svg aria-hidden width="15" height="21" viewBox="0 0 18 26">
        <path
          d={ARROW_PATH}
          className="fill-accent stroke-white"
          strokeWidth="1.6"
          strokeLinejoin="round"
        />
      </svg>
      <span className="-mt-1 ml-[11px] flex h-[19px] items-center gap-[5px] rounded-[10px] bg-ink-deep px-[7px] font-mono font-semibold text-[11px] text-white">
        <i className="size-1.5 rounded-full bg-chart-3" />
        claude
      </span>
    </div>
  )
}
