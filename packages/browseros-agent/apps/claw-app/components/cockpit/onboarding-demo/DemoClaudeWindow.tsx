/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Check,
  Code,
  MessageSquare,
  Mic,
  PanelLeft,
  Plus,
} from 'lucide-react'
import { ClaudeAiIcon } from '@/components/ui/svgs/claudeAiIcon'
import { cn } from '@/lib/utils'
import { Caret, DemoNeoMark, TrafficLights } from './demo-parts'
import { DEMO_PROMPT, type DemoFrame } from './timeline'

/** Claude's brand orange, as in its shipped mark. */
const CLAUDE_ORANGE = 'bg-[#D97757]'

const TOOLS = [
  { verb: 'navigate', arg: 'google.com/travel/flights' },
  { verb: 'fill', arg: 'New York · Fri' },
  { verb: 'read', arg: '5 flights' },
] as const

const RECENTS = ['Fix the signup bug', 'Sort my inbox', 'Tidy the README']

/**
 * The Claude desktop app in Code mode, the way most people run Claude Code
 * (owner, round 5). A simplified light drawing in neo's palette: the one sign
 * of the connection is the "BrowserOS neo ✓" chip under the composer. Tool
 * rows are two lines because one line of mono at the 11 px floor does not fit
 * this column; each row's number matches the badge it lights in neo.
 */
export function DemoClaudeWindow({
  frame,
  front,
  fading,
  spinGlyph,
}: {
  frame: DemoFrame['claude']
  front: boolean
  fading: boolean
  spinGlyph: string
}) {
  const typing = frame.composer.length > 0
  return (
    <div
      className={cn(
        'absolute top-[38px] left-[18px] flex h-[352px] w-[442px] overflow-hidden rounded-xl border border-border bg-card shadow-[0_34px_60px_-30px_rgb(1_18_63/0.45),0_8px_20px_-12px_rgb(1_18_63/0.2)] transition-[opacity,transform] duration-500 ease-out',
        front ? 'z-6' : 'z-4',
        frame.open && !fading
          ? 'opacity-100'
          : 'translate-y-3.5 scale-[0.98] opacity-0',
      )}
    >
      <div className="flex w-[134px] flex-none flex-col border-border-2 border-r bg-card-tint">
        <div className="flex h-[34px] flex-none items-center gap-2 pl-3 text-ink-4">
          <span className="mr-[3px]">
            <TrafficLights active={front} />
          </span>
          <PanelLeft aria-hidden className="size-[13px]" />
          <ArrowLeft aria-hidden className="size-3" />
          <ArrowRight aria-hidden className="size-3 opacity-55" />
        </div>
        <div className="mx-2.5 mt-0.5 mb-2 grid grid-cols-2 gap-0.5 rounded-lg bg-accent-tint p-0.5">
          <span className="flex h-[22px] items-center justify-center gap-1 rounded-md font-medium text-[11px] text-ink-3">
            <MessageSquare aria-hidden className="size-[11px]" />
            Chat
          </span>
          <span className="flex h-[22px] items-center justify-center gap-1 rounded-md bg-card font-medium text-[11px] text-cyanotype-ink shadow-[0_1px_3px_rgb(1_18_63/0.14)]">
            <Code aria-hidden className="size-[11px]" />
            Code
          </span>
        </div>
        <div
          className={cn(
            'mx-1.5 flex h-[25px] items-center gap-1.5 rounded-[7px] px-2 font-medium text-[11.5px]',
            frame.sent ? 'text-ink-2' : 'bg-accent-tint-2 text-cyanotype-ink',
          )}
        >
          <Plus aria-hidden className="size-2.5" strokeWidth={2.5} />
          New session
        </div>
        <div className="mx-3.5 mt-3.5 mb-1 text-[11px] text-ink-4 leading-3">
          Recents
        </div>
        <div
          className={cn(
            'mx-1.5 flex items-center justify-between overflow-hidden rounded-[7px] bg-accent-tint-2 px-2 font-medium text-[11px] text-cyanotype-ink transition-[height,opacity] duration-300 ease-out',
            frame.sent ? 'h-[23px] opacity-100' : 'h-0 opacity-0',
          )}
        >
          Cheapest flight
          <i
            className={cn(
              'size-1.5 flex-none rounded-full transition-opacity',
              CLAUDE_ORANGE,
              frame.working ? 'animate-pulse-dot' : 'opacity-0',
            )}
          />
        </div>
        {RECENTS.map((title) => (
          <div
            key={title}
            className="mx-1.5 flex h-[23px] items-center truncate rounded-[7px] px-2 text-[11px] text-ink-2"
          >
            {title}
          </div>
        ))}
      </div>

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-[34px] flex-none items-center px-3.5 text-[11.5px] text-ink-3">
          {frame.sent ? 'Cheapest flight' : 'New session'}
        </div>
        <div className="relative flex min-h-0 flex-1 flex-col px-3.5 pt-0.5 pb-2.5">
          <ClaudeAiIcon
            aria-hidden
            className={cn(
              'absolute top-[38%] left-1/2 -mt-3.5 -ml-3.5 size-7 transition-[opacity,transform] duration-300 ease-out',
              frame.sent && 'scale-80 opacity-0',
            )}
          />
          <div
            className={cn(
              'max-w-[88%] self-end rounded-[12px_12px_4px_12px] bg-accent-tint px-[11px] py-[7px] text-[12px] text-cyanotype-ink leading-4 transition-[opacity,transform] duration-300 ease-out',
              frame.sent ? 'opacity-100' : 'translate-y-1.5 opacity-0',
            )}
          >
            {DEMO_PROMPT}
          </div>
          <div className="mt-2.5 flex flex-col gap-[3px]">
            {TOOLS.map((tool, index) => {
              const lit = frame.litTool === index
              return (
                <div
                  key={tool.verb}
                  className={cn(
                    '-mx-1.5 grid grid-cols-[16px_minmax(0,1fr)] gap-x-2 whitespace-nowrap rounded-[7px] px-1.5 py-1 font-mono text-[11px] leading-[15px] transition-[opacity,transform,background-color] duration-300 ease-out',
                    index < frame.tools
                      ? 'opacity-100'
                      : 'translate-y-1 opacity-0',
                    lit && 'bg-accent/10',
                  )}
                >
                  <span
                    className={cn(
                      'row-span-2 -mt-px grid size-4 place-items-center rounded-full font-bold transition-colors duration-500',
                      lit
                        ? 'bg-accent text-accent-foreground'
                        : 'bg-accent-tint text-ink-3',
                    )}
                  >
                    {index + 1}
                  </span>
                  <span className="text-ink-2">
                    browseros-neo ·{' '}
                    <b className="font-semibold text-cyanotype-ink">
                      {tool.verb}
                    </b>
                  </span>
                  <span
                    className={cn(
                      'transition-colors duration-500',
                      lit ? 'text-cyanotype-ink' : 'text-ink-3',
                    )}
                  >
                    {tool.arg}
                  </span>
                </div>
              )
            })}
          </div>
          <div className="mt-[9px] grid">
            <div
              className={cn(
                'col-start-1 row-start-1 flex items-center gap-[7px] text-[11px] text-ink-3 leading-4 transition-opacity duration-400',
                frame.working ? 'opacity-100' : 'opacity-0',
              )}
            >
              <span className="w-3 text-center font-mono font-semibold text-[#D97757] text-[12px]">
                {spinGlyph}
              </span>
              Browsing… ({frame.seconds}s)
            </div>
            <div
              className={cn(
                'col-start-1 row-start-1 flex items-center gap-[7px] font-medium text-[12px] text-cyanotype-ink leading-4 transition-opacity duration-400',
                frame.answered ? 'opacity-100' : 'opacity-0',
              )}
            >
              <span className="grid size-4 flex-none place-items-center rounded-full bg-accent text-accent-foreground">
                <Check aria-hidden className="size-2.5" strokeWidth={3} />
              </span>
              Cheapest is $142, nonstop, Fri 7:05 am.
            </div>
          </div>
          <div className="mt-auto flex items-start gap-2 rounded-xl border border-border bg-card p-2.5 text-[12px] leading-4 shadow-[0_2px_8px_-4px_rgb(1_18_63/0.14)]">
            <Plus aria-hidden className="mt-0.5 size-3 flex-none text-ink-3" />
            <span className="min-w-0 flex-1">
              {typing ? (
                <span className="text-cyanotype-ink">
                  {frame.composer}
                  {front && <Caret className="mx-px h-3.5" />}
                </span>
              ) : (
                <span className="text-ink-4">
                  Describe a task or ask a question
                </span>
              )}
            </span>
            {typing ? (
              <span
                className={cn(
                  '-my-0.5 -mr-0.5 grid size-5 flex-none place-items-center self-end rounded-full text-white',
                  CLAUDE_ORANGE,
                )}
              >
                <ArrowUp aria-hidden className="size-2.5" strokeWidth={2.5} />
              </span>
            ) : (
              <Mic aria-hidden className="mt-0.5 size-3 flex-none text-ink-3" />
            )}
          </div>
          <span className="mt-[7px] inline-flex h-5 items-center gap-[5px] self-start rounded-[10px] border border-border-2 bg-card-tint pr-2 pl-1 font-medium text-[11px] text-ink-2">
            <DemoNeoMark className="size-[13px] rounded-[3px]" />
            BrowserOS neo
            <Check
              aria-hidden
              className="size-2.5 text-accent"
              strokeWidth={3}
            />
          </span>
        </div>
      </div>
    </div>
  )
}
