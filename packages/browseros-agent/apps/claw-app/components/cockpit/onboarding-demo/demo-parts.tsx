/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Small pieces shared by the demo's drawn windows. Sizes are stage pixels on
 * the 960×480 desktop; the player scales the whole stage, so nothing here may
 * go below 11 px or it reads under 11 px at the cockpit's 1040 width.
 */

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
