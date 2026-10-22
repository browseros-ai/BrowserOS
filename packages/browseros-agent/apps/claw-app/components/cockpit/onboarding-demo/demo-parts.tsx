/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Small pieces shared by the demo's drawn windows. Sizes are stage pixels on
 * the 960×480 desktop; the player scales the whole stage, so nothing here may
 * go below 11 px or it reads under 11 px at the cockpit's 1040 width.
 */

import { Check } from 'lucide-react'
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

/** macOS window controls; only the frontmost window shows them in colour. */
export function TrafficLights({ active }: { active: boolean }) {
  const colors = active
    ? ['bg-[#FF5F57]', 'bg-[#FEBC2E]', 'bg-[#28C840]']
    : ['bg-border', 'bg-border', 'bg-border']
  return (
    <span className="flex flex-none gap-[7px]">
      {colors.map((color, index) => (
        <i
          // biome-ignore lint/suspicious/noArrayIndexKey: three fixed lights
          key={index}
          className={cn(
            'size-[11px] rounded-full transition-colors duration-300',
            color,
          )}
        />
      ))}
    </span>
  )
}

/** The numbered marker a tool row lights on the part of the page it touches. */
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
        'grid size-5 flex-none place-items-center rounded-full bg-accent font-bold font-mono text-[11px] text-accent-foreground leading-none shadow-[0_0_0_2px_var(--color-card),0_3px_8px_rgb(2_84_236/0.35)] transition-[opacity,transform] duration-300 ease-out',
        state === 'off' ? 'scale-40 opacity-0' : 'opacity-100',
        state === 'live' && 'animate-demo-halo',
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
        'inline-block w-[1.5px] animate-demo-blink bg-current align-[-2px]',
        className,
      )}
    />
  )
}

const TOOL_CALLS = [
  { verb: 'navigate', arg: 'google.com/travel/flights' },
  { verb: 'fill', arg: 'New York · Fri' },
  { verb: 'read', arg: '5 flights' },
] as const

/** Each app window's own colours for the tool rows (literal for Tailwind). */
const TOOL_TONES = {
  claude: {
    lit: 'bg-(--claude-selected)',
    idleNumber: 'bg-(--claude-toggle) text-(--claude-muted)',
    name: 'text-(--claude-text)',
    verb: 'text-(--claude-ink)',
    litArg: 'text-(--claude-ink)',
    arg: 'text-(--claude-muted)',
  },
  chatgpt: {
    lit: 'bg-(--chatgpt-selected)',
    idleNumber: 'bg-(--chatgpt-toggle) text-(--chatgpt-muted)',
    name: 'text-(--chatgpt-text)',
    verb: 'text-(--chatgpt-ink)',
    litArg: 'text-(--chatgpt-ink)',
    arg: 'text-(--chatgpt-muted)',
  },
} as const

/**
 * The agent's three browseros-neo calls as two-line rows (name, then
 * argument): one line of mono at the 11 px floor does not fit these columns.
 * Each number matches the badge it lights in neo, so it stays neo blue.
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
    <div className="mt-2.5 flex flex-col gap-[3px]">
      {TOOL_CALLS.map((tool, index) => {
        const isLit = lit === index
        return (
          <div
            key={tool.verb}
            className={cn(
              '-mx-1.5 grid grid-cols-[16px_minmax(0,1fr)] gap-x-2 whitespace-nowrap rounded-[7px] px-1.5 py-1 font-mono text-[11px] leading-[15px] transition-[opacity,transform,background-color] duration-300 ease-out',
              index < shown ? 'opacity-100' : 'translate-y-1 opacity-0',
              isLit && colors.lit,
            )}
          >
            <span
              className={cn(
                'row-span-2 -mt-px grid size-4 place-items-center rounded-full font-bold transition-colors duration-500',
                isLit ? 'bg-accent text-accent-foreground' : colors.idleNumber,
              )}
            >
              {index + 1}
            </span>
            <span className={colors.name}>
              browseros-neo ·{' '}
              <b className={cn('font-semibold', colors.verb)}>{tool.verb}</b>
            </span>
            <span
              className={cn(
                'transition-colors duration-500',
                isLit ? colors.litArg : colors.arg,
              )}
            >
              {tool.arg}
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
        'mt-[7px] inline-flex h-5 items-center gap-[5px] self-start rounded-[10px] border pr-2 pl-1 font-medium text-[11px] transition-opacity duration-300',
        className,
      )}
    >
      <DemoNeoMark className="size-[13px] rounded-[3px]" />
      BrowserOS neo
      <Check aria-hidden className="size-2.5 opacity-70" strokeWidth={3} />
    </span>
  )
}
