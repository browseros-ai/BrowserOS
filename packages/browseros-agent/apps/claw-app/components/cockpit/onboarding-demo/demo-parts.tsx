/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Small pieces shared by the demo's drawn windows. Sizes are stage pixels on
 * Paper's 976×520 cockpit stage; the player scales the whole stage to the
 * column, so nothing here may go below 11 px. The stage is a drawing of a
 * light macOS desktop, so its colours are literal and stay light in neo's Gray
 * theme, like the apps it draws.
 */

import { Check } from 'lucide-react'
import { type ReactNode, useLayoutEffect, useRef, useState } from 'react'
import { cn } from '@/lib/utils'
import type { BadgeState } from './timeline'

/** neo's dog mark: the same asset the sidebar shows. */
export function DemoNeoMark({ className }: { className?: string }) {
  return (
    <img
      src="/icons/browserclaw.svg"
      alt=""
      aria-hidden
      draggable={false}
      className={cn('shrink-0 select-none', className)}
    />
  )
}

/**
 * macOS window controls, in colour on every window (Paper): the windows sit
 * side by side, so none of them is ever dimmed behind another. Sidebar apps
 * (Claude, ChatGPT) draw them at 10 px, the browser and Terminal at 11 px.
 */
export function TrafficLights({ size }: { size: 10 | 11 }) {
  return (
    <span
      className={cn('flex flex-none', size === 10 ? 'gap-1.5' : 'gap-[7px]')}
    >
      {['bg-[#FF5F57]', 'bg-[#FEBC2E]', 'bg-[#28C840]'].map((color) => (
        <i
          key={color}
          className={cn(
            'rounded-full',
            size === 10 ? 'size-2.5' : 'size-[11px]',
            color,
          )}
        />
      ))}
    </span>
  )
}

/**
 * The numbered marker a tool row lights on the part of the page it touches:
 * a white-ringed neo-blue dot, with a soft halo while its row is lit.
 */
export function StepBadge({
  n,
  state,
  className,
}: {
  n: number
  state: BadgeState
  className?: string
}) {
  return (
    <span
      className={cn(
        'grid size-5 flex-none place-items-center rounded-full bg-[#0254EC] font-bold font-mono text-[11px] text-white leading-3 transition-[opacity,transform,box-shadow] duration-300 ease-out',
        state === 'off' ? 'scale-60 opacity-0' : 'opacity-100',
        state === 'live'
          ? 'shadow-[0_0_0_2px_#fff,0_0_0_5px_rgb(2_84_236/0.22)]'
          : 'shadow-[0_0_0_2px_#fff]',
        className,
      )}
    >
      {n}
    </span>
  )
}

/** A blinking text caret for whatever is being typed into. */
export function Caret({ className }: { className?: string }) {
  return (
    <i
      aria-hidden
      className={cn(
        'inline-block w-[1.5px] animate-demo-blink bg-current align-[-3px]',
        className,
      )}
    />
  )
}

/** The macOS arrow, for your pointer and the agent's (Paper: 18×22). */
export function PointerArrow({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden
      width="18"
      height="22"
      viewBox="0 0 18 22"
      className={className}
    >
      <path
        d="M2 1.5v16.2l4.4-4.1 2.9 6.6 2.7-1.2-2.9-6.5H15z"
        fill="#01123F"
        stroke="#fff"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
    </svg>
  )
}

/** The three browseros-neo calls every flow shows, with what each touched. */
export const TOOL_CALLS = [
  { name: 'browseros-neo · navigate', arg: 'google.com/travel/flights' },
  { name: 'browseros-neo · fill', arg: 'New York · Fri' },
  { name: 'browseros-neo · read', arg: '5 flights' },
] as const

/** Each app window's own colours for the tool rows (literal for Tailwind). */
const TOOL_TONES = {
  claude: {
    lit: 'border-[#E4E4E3] bg-[#F0EFED]',
    halo: 'shadow-[0_0_0_2px_rgb(2_84_236/0.2)]',
    name: 'text-[#1F1E1D]',
    arg: 'text-[#8A8781]',
    litArg: 'text-[#52514F]',
  },
  chatgpt: {
    lit: 'border-[#4B4C55] bg-[#393A45]',
    halo: 'shadow-[0_0_0_2px_rgb(2_84_236/0.25)]',
    name: 'text-[#F8F8F2]',
    arg: 'text-[#A2A2A4]',
    litArg: 'text-[#D1D1CF]',
  },
} as const

/**
 * The agent's three calls as two-line rows (name, then argument), the way the
 * Claude and ChatGPT apps list tool use. Each number matches the badge it
 * lights in neo, so it stays neo blue: dimmed once done, solid while lit.
 * A row exists only once its call has started, so it rises in.
 */
export function DemoToolRows({
  shown,
  lit,
  tone,
}: {
  shown: number
  lit: number | null
  tone: keyof typeof TOOL_TONES
}) {
  const colors = TOOL_TONES[tone]
  return (
    <div className="flex flex-col gap-0.5">
      {TOOL_CALLS.slice(0, shown).map((tool, index) => {
        const isLit = lit === index
        return (
          <div
            key={tool.name}
            className={cn(
              'flex animate-demo-rise items-start gap-[9px] whitespace-nowrap rounded-[7px] border px-2 py-1 font-mono transition-[background-color,border-color] duration-500 ease-out',
              isLit ? colors.lit : 'border-transparent',
            )}
          >
            <span
              className={cn(
                'grid size-[18px] flex-none place-items-center rounded-full font-bold text-[11px] text-white leading-3 transition-[background-color,box-shadow] duration-500',
                isLit ? cn('bg-[#0254EC]', colors.halo) : 'bg-[#0254EC]/60',
              )}
            >
              {index + 1}
            </span>
            <span className="flex flex-col gap-px">
              <span className={cn('text-[11.5px] leading-4', colors.name)}>
                {tool.name}
              </span>
              <span
                className={cn(
                  'text-[11px] leading-[15px] transition-colors duration-500',
                  isLit ? colors.litArg : colors.arg,
                )}
              >
                {tool.arg}
              </span>
            </span>
          </div>
        )
      })}
    </div>
  )
}

/** The one sign of the connection in an app window: "BrowserOS neo ✓". */
export function NeoConnectedChip({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        'inline-flex h-[22px] flex-none items-center gap-1.5 self-start rounded-full pr-2 pl-1 font-medium text-[11.5px] leading-[14px]',
        className,
      )}
    >
      <DemoNeoMark className="size-3.5 rounded-[4px]" />
      BrowserOS neo
      <Check aria-hidden className="size-[11px]" strokeWidth={2.5} />
    </span>
  )
}

/**
 * A transcript that scrolls up just far enough to show its last line, as the
 * apps do once the answer lands. The cockpit windows fit every line through
 * Watch, so the person's message stays in view; only the answer can push it
 * up. It measures after every render (layout px, unaffected by the stage's
 * scale) and slides by a transform, so Pause freezes the slide mid-way.
 */
export function Transcript({
  gap,
  children,
}: {
  /** Space between lines, in px. */
  gap: number
  children: ReactNode
}) {
  const boxRef = useRef<HTMLDivElement>(null)
  const [offset, setOffset] = useState(0)
  useLayoutEffect(() => {
    const box = boxRef.current
    const content = box?.firstElementChild
    if (!box || !(content instanceof HTMLElement)) return
    const next = Math.max(0, content.offsetHeight - box.clientHeight)
    if (next !== offset) setOffset(next)
  })
  return (
    <div
      ref={boxRef}
      className={cn(
        'min-h-0 flex-1 overflow-hidden',
        offset > 0 && 'mask-[linear-gradient(180deg,transparent_0,#000_14px)]',
      )}
    >
      <div
        style={{ gap, transform: `translateY(${-offset}px)` }}
        className="flex flex-col transition-transform duration-500 ease-out"
      >
        {children}
      </div>
    </div>
  )
}
