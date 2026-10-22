/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Check } from 'lucide-react'
import { cn } from '@/lib/utils'
import { Caret, StepBadge, TrafficLights } from './demo-parts'
import { DEMO_PROMPT, type DemoFrame } from './timeline'

const TOOL_LINES = [
  { verb: 'navigate', arg: 'google.com/travel/flights' },
  { verb: 'fill', arg: 'New York · Fri' },
  { verb: 'read', arg: '5 flights' },
] as const

/**
 * Claude Code in a terminal, in round 4's look: type `claude` and press
 * return, Claude Code starts and shows it is connected to neo, then the ask
 * goes in at its `>` prompt (round 10). The window is wide enough for one
 * line per tool call; each number lights its badge in neo.
 */
export function DemoTerminalWindow({
  frame,
  front,
  fading,
  spinGlyph,
}: {
  frame: DemoFrame['agent']
  front: boolean
  fading: boolean
  spinGlyph: string
}) {
  const started = frame.switched
  const typing = frame.composer.length > 0
  return (
    <div
      className={cn(
        'absolute top-3 left-[18px] flex h-[378px] w-[442px] flex-col overflow-hidden rounded-xl border border-white/10 bg-ink-deep font-mono text-[12px] text-white/80 leading-[18px] shadow-[0_34px_60px_-30px_rgb(1_18_63/0.6),0_8px_20px_-12px_rgb(1_18_63/0.3)] transition-[opacity,transform] duration-500 ease-out',
        front ? 'z-6' : 'z-4',
        frame.open && !fading
          ? 'opacity-100'
          : 'translate-y-3.5 scale-[0.98] opacity-0',
      )}
    >
      <div className="relative flex h-[30px] flex-none items-center border-white/10 border-b px-3">
        <TrafficLights active={front} />
        <span className="absolute inset-x-0 text-center text-[11px] text-white/45">
          {started ? 'claude — ~' : 'zsh — ~'}
        </span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col gap-2.5 p-3.5">
        <div>
          <span className="text-white/45">~ % </span>
          <span className="text-white">{frame.command}</span>
          {!started && front && <Caret className="ml-px h-3.5 text-white" />}
        </div>
        <div
          className={cn(
            'rounded-lg border border-chart-3/50 px-3 py-2 transition-[opacity,transform] duration-300 ease-out',
            started ? 'opacity-100' : 'translate-y-1 opacity-0',
          )}
        >
          <div className="flex items-center gap-2 font-semibold text-[13px] text-white">
            <span className="text-chart-3">✻</span>
            Claude Code
          </div>
          <div className="text-[11px] text-white/50">
            browseros-neo <span className="text-white/80">✓</span> connected
          </div>
        </div>
        <div
          className={cn(
            'text-white transition-opacity duration-300',
            frame.sent ? 'opacity-100' : 'opacity-0',
          )}
        >
          &gt; {DEMO_PROMPT}
        </div>
        <div className="flex flex-col gap-1">
          {TOOL_LINES.map((tool, index) => {
            const lit = frame.litTool === index
            return (
              <div
                key={tool.verb}
                className={cn(
                  '-mx-1.5 flex items-center gap-2 whitespace-nowrap rounded-md border px-1.5 py-[3px] text-[11px] transition-[opacity,transform,background-color,border-color] duration-300 ease-out',
                  index < frame.tools
                    ? 'opacity-100'
                    : 'translate-y-1 opacity-0',
                  lit ? 'border-accent/70 bg-accent/20' : 'border-transparent',
                )}
              >
                <StepBadge
                  n={index + 1}
                  state={lit ? 'on' : 'off'}
                  className={cn(
                    'size-4 text-[11px] shadow-none',
                    !lit && 'scale-100 bg-white/15 opacity-100',
                  )}
                />
                <span className="text-white/85">
                  browseros-neo · <b className="font-semibold">{tool.verb}</b>
                </span>
                <span className="text-white/45">{tool.arg}</span>
              </div>
            )
          })}
        </div>
        <div className="grid">
          <div
            className={cn(
              'col-start-1 row-start-1 flex items-center gap-2 text-[11px] text-chart-3 transition-opacity duration-400',
              frame.working ? 'opacity-100' : 'opacity-0',
            )}
          >
            <span className="w-3 text-center font-semibold">{spinGlyph}</span>
            Browsing… ({frame.seconds}s)
          </div>
          <div
            className={cn(
              'col-start-1 row-start-1 flex items-center gap-2 font-medium font-sans text-[12.5px] text-white transition-opacity duration-400',
              frame.answered ? 'opacity-100' : 'opacity-0',
            )}
          >
            <span className="grid size-4 flex-none place-items-center rounded-full bg-accent text-accent-foreground">
              <Check aria-hidden className="size-2.5" strokeWidth={3} />
            </span>
            Cheapest is $142, nonstop, Fri 7:05 am.
          </div>
        </div>
        <div
          className={cn(
            'mt-auto flex min-h-9 items-start rounded-lg border border-white/15 px-2.5 py-2 transition-opacity duration-300',
            started ? 'opacity-100' : 'opacity-0',
          )}
        >
          <span className="text-white/45">&gt;&nbsp;</span>
          <span className="min-w-0 flex-1 text-white">
            {frame.composer}
            {started && front && !frame.sent && (
              <Caret className={cn('h-3.5', typing && 'ml-px')} />
            )}
          </span>
        </div>
      </div>
    </div>
  )
}
