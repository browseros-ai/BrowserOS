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
import { CLAUDE_LIGHT } from './claude-light'
import { Caret, DemoNeoMark, TrafficLights } from './demo-parts'
import { DEMO_PROMPT, type DemoFrame } from './timeline'

/** Claude's brand orange: the send button, the running dot, the spinner. */
const CLAUDE_ORANGE = 'bg-(--claude-orange)'

const TOOLS = [
  { verb: 'navigate', arg: 'google.com/travel/flights' },
  { verb: 'fill', arg: 'New York · Fri' },
  { verb: 'read', arg: '5 flights' },
] as const

const RECENTS = ['Fix the signup bug', 'Sort my inbox', 'Tidy the README']

/**
 * The Claude desktop app in Code mode, the way most people run Claude Code
 * (owner, round 5), in Claude's own default light colors (round 8). A
 * simplified drawing: the one sign
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
      style={CLAUDE_LIGHT}
      className={cn(
        'absolute top-3 left-[18px] flex h-[378px] w-[442px] overflow-hidden rounded-xl border border-(--claude-border) bg-(--claude-main) shadow-[0_34px_60px_-30px_rgb(20_20_19/0.4),0_8px_20px_-12px_rgb(20_20_19/0.18)] transition-[opacity,transform] duration-500 ease-out',
        front ? 'z-6' : 'z-4',
        frame.open && !fading
          ? 'opacity-100'
          : 'translate-y-3.5 scale-[0.98] opacity-0',
      )}
    >
      <div className="flex w-[134px] flex-none flex-col border-(--claude-border) border-r bg-(--claude-sidebar)">
        <div className="flex h-[34px] flex-none items-center gap-2 pl-3 text-(--claude-muted)">
          <span className="mr-[3px]">
            <TrafficLights active={front} />
          </span>
          <PanelLeft aria-hidden className="size-[13px]" />
          <ArrowLeft aria-hidden className="size-3" />
          <ArrowRight aria-hidden className="size-3 opacity-55" />
        </div>
        <div className="mx-2.5 mt-0.5 mb-2 grid grid-cols-2 gap-0.5 rounded-lg bg-(--claude-toggle) p-0.5">
          <span className="flex h-[22px] items-center justify-center gap-1 rounded-md font-medium text-(--claude-muted) text-[11px]">
            <MessageSquare aria-hidden className="size-[11px]" />
            Chat
          </span>
          <span className="flex h-[22px] items-center justify-center gap-1 rounded-md bg-white font-medium text-(--claude-ink) text-[11px] shadow-[0_1px_2px_rgb(0_0_0/0.12)]">
            <Code aria-hidden className="size-[11px]" />
            Code
          </span>
        </div>
        <div
          className={cn(
            'mx-1.5 flex h-[25px] items-center gap-1.5 rounded-[7px] px-2 font-medium text-[11.5px]',
            frame.sent
              ? 'text-(--claude-text)'
              : 'bg-(--claude-selected) text-(--claude-ink)',
          )}
        >
          <Plus aria-hidden className="size-2.5" strokeWidth={2.5} />
          New session
        </div>
        <div className="mx-3.5 mt-3.5 mb-1 text-(--claude-muted) text-[11px] leading-3">
          Recents
        </div>
        <div
          className={cn(
            'mx-1.5 flex items-center justify-between overflow-hidden rounded-[7px] bg-(--claude-selected) px-2 font-medium text-(--claude-ink) text-[11px] transition-[height,opacity] duration-300 ease-out',
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
            className="mx-1.5 flex h-[23px] items-center truncate rounded-[7px] px-2 text-(--claude-text) text-[11px]"
          >
            {title}
          </div>
        ))}
      </div>

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-[34px] flex-none items-center px-3.5 text-(--claude-muted) text-[11.5px]">
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
              'max-w-[88%] self-end rounded-[12px_12px_4px_12px] bg-(--claude-bubble) px-[11px] py-[7px] text-(--claude-ink) text-[13px] leading-[17px] transition-[opacity,transform] duration-300 ease-out',
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
                    lit && 'bg-(--claude-selected)',
                  )}
                >
                  <span
                    className={cn(
                      'row-span-2 -mt-px grid size-4 place-items-center rounded-full font-bold transition-colors duration-500',
                      // The numbers are neo's badges: the one place neo blue
                      // shows inside Claude's window.
                      lit
                        ? 'bg-accent text-accent-foreground'
                        : 'bg-(--claude-toggle) text-(--claude-muted)',
                    )}
                  >
                    {index + 1}
                  </span>
                  <span className="text-(--claude-text)">
                    browseros-neo ·{' '}
                    <b className="font-semibold text-(--claude-ink)">
                      {tool.verb}
                    </b>
                  </span>
                  <span
                    className={cn(
                      'transition-colors duration-500',
                      lit ? 'text-(--claude-ink)' : 'text-(--claude-muted)',
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
                'col-start-1 row-start-1 flex items-center gap-[7px] text-(--claude-muted) text-[11px] leading-4 transition-opacity duration-400',
                frame.working ? 'opacity-100' : 'opacity-0',
              )}
            >
              <span className="w-3 text-center font-mono font-semibold text-(--claude-orange) text-[12px]">
                {spinGlyph}
              </span>
              Browsing… ({frame.seconds}s)
            </div>
            <div
              className={cn(
                'col-start-1 row-start-1 flex items-center gap-[7px] font-medium text-(--claude-ink) text-[12px] leading-4 transition-opacity duration-400',
                frame.answered ? 'opacity-100' : 'opacity-0',
              )}
            >
              <span className="grid size-4 flex-none place-items-center rounded-full bg-(--claude-ink) text-white">
                <Check aria-hidden className="size-2.5" strokeWidth={3} />
              </span>
              Cheapest is $142, nonstop, Fri 7:05 am.
            </div>
          </div>
          <div className="mt-auto flex items-start gap-2 rounded-xl border border-(--claude-composer-border) bg-white p-2.5 text-[13px] leading-[17px] shadow-[0_2px_8px_-4px_rgb(0_0_0/0.1)]">
            <Plus
              aria-hidden
              className="mt-0.5 size-3 flex-none text-(--claude-muted)"
            />
            <span className="min-w-0 flex-1">
              {typing ? (
                <span className="text-(--claude-ink)">
                  {frame.composer}
                  {front && <Caret className="mx-px h-3.5" />}
                </span>
              ) : (
                <span className="text-(--claude-muted)">
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
              <Mic
                aria-hidden
                className="mt-0.5 size-3 flex-none text-(--claude-muted)"
              />
            )}
          </div>
          <span className="mt-[7px] inline-flex h-5 items-center gap-[5px] self-start rounded-[10px] border border-(--claude-border) bg-(--claude-sidebar) pr-2 pl-1 font-medium text-(--claude-text) text-[11px]">
            <DemoNeoMark className="size-[13px] rounded-[3px]" />
            BrowserOS neo
            <Check
              aria-hidden
              className="size-2.5 text-(--claude-muted)"
              strokeWidth={3}
            />
          </span>
        </div>
      </div>
    </div>
  )
}
