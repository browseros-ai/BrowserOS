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

/** macOS draws its own chrome in the system face, not the app's. */
const MAC_FONT =
  "font-[-apple-system,BlinkMacSystemFont,'SF_Pro_Text','Helvetica_Neue',sans-serif]"

const NEO_MENUS = [
  'File',
  'Edit',
  'View',
  'Agents',
  'Sessions',
  'Window',
  'Help',
]
const CLAUDE_MENUS = ['File', 'Edit', 'View', 'Window', 'Help']

/** The 960×480 drawn desktop at one frame of the timeline. */
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
      {/* The dithered neo-blue wallpaper from the website's v3-C hero. */}
      <div className="absolute inset-0 bg-[url(/onboarding/demo-wallpaper.png)] bg-accent-tint bg-size-[1200px_680px] [image-rendering:pixelated] dark:opacity-30" />
      <MenuBar claudeFront={claudeFront} working={frame.menu.agentWorking} />
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

/**
 * Names the frontmost app with its menus. The first slot (Apple's, on a real
 * Mac) is neo's dog only while neo is in front, a neutral glyph otherwise; no
 * Apple logo is drawn. "1 agent working" is neo's own menu-bar item.
 */
function MenuBar({
  claudeFront,
  working,
}: {
  claudeFront: boolean
  working: boolean
}) {
  return (
    <div
      className={cn(
        'absolute inset-x-0 top-0 z-8 flex h-[26px] items-center whitespace-nowrap border-cyanotype-ink/5 border-b bg-card/75 px-3.5 text-[13px] text-cyanotype-ink leading-[26px] backdrop-blur-md',
        MAC_FONT,
      )}
    >
      <span className="flex items-center gap-[18px]">
        <span className="flex items-center gap-[7px] font-bold">
          {claudeFront ? (
            <i className="mx-px size-[13px] rounded-[4px] bg-ink-3" />
          ) : (
            <DemoNeoMark className="size-[15px] rounded-[4px]" />
          )}
          {claudeFront ? 'Claude' : 'BrowserOS neo'}
        </span>
        {(claudeFront ? CLAUDE_MENUS : NEO_MENUS).map((menu) => (
          <span key={menu}>{menu}</span>
        ))}
      </span>
      <span className="ml-auto flex items-center gap-3.5 text-[12.5px]">
        <span
          className={cn(
            'flex items-center gap-1.5 transition-opacity duration-300',
            working ? 'opacity-100' : 'opacity-0',
          )}
        >
          <i className="size-[7px] animate-pulse-dot rounded-full bg-chart-3" />
          1 agent working
        </span>
        <svg
          aria-hidden
          width="16"
          height="12"
          viewBox="0 0 16 12"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.5"
          strokeLinecap="round"
        >
          <path d="M1.5 4.2a9.5 9.5 0 0 1 13 0M3.8 6.7a6.2 6.2 0 0 1 8.4 0M6.1 9.1a2.9 2.9 0 0 1 3.8 0" />
        </svg>
        <svg aria-hidden width="24" height="12" viewBox="0 0 24 12" fill="none">
          <rect
            x=".75"
            y=".75"
            width="19.5"
            height="10.5"
            rx="3"
            stroke="currentColor"
            strokeOpacity=".55"
            strokeWidth="1.2"
          />
          <rect
            x="2.5"
            y="2.5"
            width="13"
            height="7"
            rx="1.6"
            fill="currentColor"
          />
          <path
            d="M22 4.2v3.6"
            stroke="currentColor"
            strokeOpacity=".55"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
        <span>Thu Oct 22&nbsp;&nbsp;9:41 AM</span>
      </span>
    </div>
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
  off: 'top-[220px] left-[760px]',
  to: 'top-[156px] left-[738px]',
  when: 'top-[156px] left-[872px]',
  answer: 'top-[300px] left-[784px]',
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
