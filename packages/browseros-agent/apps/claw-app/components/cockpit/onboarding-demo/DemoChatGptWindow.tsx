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
  ChevronDown,
  Mic,
  PanelLeft,
  Plus,
  SquarePen,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import { CHATGPT_DARK } from './chatgpt-dark'
import {
  Caret,
  DemoToolRows,
  NeoConnectedChip,
  TrafficLights,
} from './demo-parts'
import { DEMO_PROMPT, type DemoFrame } from './timeline'

const RECENTS = {
  chat: ['Weekend trip ideas', 'Thank-you note draft'],
  work: ['Fix the signup bug', 'Sort my inbox', 'Tidy the README'],
} as const

/**
 * The ChatGPT desktop app in its dark theme, drawn from the owner's
 * screenshot (round 10): Codex now runs inside it, behind the Work half of
 * the Chat | Work toggle at the top. It opens on Chat; the Switch beat moves
 * it to Work. Simplified like the Claude window: no projects, no recap card,
 * no model picker, and the composer stays at the bottom throughout.
 */
export function DemoChatGptWindow({
  frame,
  front,
  fading,
}: {
  frame: DemoFrame['agent']
  front: boolean
  fading: boolean
}) {
  const typing = frame.composer.length > 0
  const work = frame.switched
  return (
    <div
      style={CHATGPT_DARK}
      className={cn(
        'absolute top-3 left-[18px] flex h-[378px] w-[442px] overflow-hidden rounded-xl border border-(--chatgpt-border) bg-(--chatgpt-main) text-(--chatgpt-text) shadow-[0_34px_60px_-30px_rgb(0_0_0/0.6),0_8px_20px_-12px_rgb(0_0_0/0.35)] transition-[opacity,transform] duration-500 ease-out',
        front ? 'z-6' : 'z-4',
        frame.open && !fading
          ? 'opacity-100'
          : 'translate-y-3.5 scale-[0.98] opacity-0',
      )}
    >
      <div className="flex w-[134px] flex-none flex-col bg-(--chatgpt-sidebar)">
        <div className="flex h-[34px] flex-none items-center gap-2 pl-3 text-(--chatgpt-muted)">
          <span className="mr-[3px]">
            <TrafficLights active={front} />
          </span>
          <ArrowLeft aria-hidden className="size-3" />
          <ArrowRight aria-hidden className="size-3 opacity-55" />
          <PanelLeft aria-hidden className="size-[13px]" />
        </div>
        <div className="mx-3 mt-1 mb-2 flex items-center gap-1 font-semibold text-(--chatgpt-ink) text-[13px]">
          ChatGPT
          <ChevronDown aria-hidden className="size-3 text-(--chatgpt-muted)" />
        </div>
        <div
          className={cn(
            'mx-1.5 flex h-[25px] items-center gap-1.5 rounded-[7px] px-2 text-(--chatgpt-ink) text-[11.5px]',
            !frame.sent && 'bg-(--chatgpt-selected)',
          )}
        >
          <SquarePen aria-hidden className="size-3" />
          New chat
        </div>
        <div className="mx-3.5 mt-3.5 mb-1 text-(--chatgpt-muted) text-[11px] leading-3">
          Recents
        </div>
        <div
          className={cn(
            'mx-1.5 flex items-center justify-between overflow-hidden rounded-[7px] bg-(--chatgpt-selected) px-2 text-(--chatgpt-ink) text-[11px] transition-[height,opacity] duration-300 ease-out',
            frame.sent ? 'h-[23px] opacity-100' : 'h-0 opacity-0',
          )}
        >
          Cheapest flight
          <i
            className={cn(
              'size-1.5 flex-none rounded-full bg-(--chatgpt-ink) transition-opacity',
              frame.working ? 'animate-pulse-dot' : 'opacity-0',
            )}
          />
        </div>
        {RECENTS[work ? 'work' : 'chat'].map((title) => (
          <div
            key={title}
            className="mx-1.5 flex h-[23px] items-center truncate rounded-[7px] px-2 text-(--chatgpt-text) text-[11px]"
          >
            {title}
          </div>
        ))}
      </div>

      <div className="flex min-w-0 flex-1 flex-col">
        {/* Chat | Work: the lighter thumb slides to Work on the Switch beat. */}
        <div className="flex h-[44px] flex-none items-center justify-center">
          <div className="relative grid w-[124px] grid-cols-2 rounded-full bg-(--chatgpt-toggle) p-0.5">
            <i
              className={cn(
                'absolute top-0.5 bottom-0.5 left-0.5 w-[calc(50%-2px)] rounded-full border border-white/10 bg-(--chatgpt-toggle-on) transition-transform duration-300 ease-out',
                work && 'translate-x-full',
              )}
            />
            {(['Chat', 'Work'] as const).map((label) => (
              <span
                key={label}
                className={cn(
                  'relative flex h-[22px] items-center justify-center text-[11.5px] transition-colors duration-300',
                  (label === 'Work') === work
                    ? 'text-(--chatgpt-ink)'
                    : 'text-(--chatgpt-muted)',
                )}
              >
                {label}
              </span>
            ))}
          </div>
        </div>
        <div className="relative flex min-h-0 flex-1 flex-col px-3.5 pt-0.5 pb-2.5">
          <div
            className={cn(
              'absolute inset-x-3.5 top-[34%] text-center font-medium text-(--chatgpt-ink) text-[17px] tracking-[-0.01em] transition-opacity duration-300',
              frame.sent && 'opacity-0',
            )}
          >
            {work ? 'What should we work on?' : 'What can I help with?'}
          </div>
          <div
            className={cn(
              'max-w-[88%] self-end rounded-[14px] bg-(--chatgpt-composer) px-[11px] py-[7px] text-(--chatgpt-ink) text-[13px] leading-[17px] transition-[opacity,transform] duration-300 ease-out',
              frame.sent ? 'opacity-100' : 'translate-y-1.5 opacity-0',
            )}
          >
            {DEMO_PROMPT}
          </div>
          <DemoToolRows
            shown={frame.tools}
            lit={frame.litTool}
            tone="chatgpt"
          />
          <div className="mt-[9px] grid">
            <div
              className={cn(
                'col-start-1 row-start-1 flex items-center gap-[7px] text-(--chatgpt-muted) text-[11px] leading-4 transition-opacity duration-400',
                frame.working ? 'opacity-100' : 'opacity-0',
              )}
            >
              <i className="ml-1 size-2 flex-none animate-pulse-dot rounded-full bg-(--chatgpt-ink)" />
              Browsing… ({frame.seconds}s)
            </div>
            <div
              className={cn(
                'col-start-1 row-start-1 flex items-center gap-[7px] font-medium text-(--chatgpt-ink) text-[12px] leading-4 transition-opacity duration-400',
                frame.answered ? 'opacity-100' : 'opacity-0',
              )}
            >
              <span className="grid size-4 flex-none place-items-center rounded-full bg-(--chatgpt-ink) text-(--chatgpt-main)">
                <Check aria-hidden className="size-2.5" strokeWidth={3} />
              </span>
              Cheapest is $142, nonstop, Fri 7:05 am.
            </div>
          </div>
          <div className="mt-auto flex items-start gap-2 rounded-[16px] bg-(--chatgpt-composer) p-2.5 text-[13px] leading-[17px]">
            <Plus
              aria-hidden
              className="mt-0.5 size-3 flex-none text-(--chatgpt-muted)"
            />
            <span className="min-w-0 flex-1">
              {typing ? (
                <span className="text-(--chatgpt-ink)">
                  {frame.composer}
                  {front && <Caret className="mx-px h-3.5" />}
                </span>
              ) : (
                <span className="text-(--chatgpt-muted)">Ask anything</span>
              )}
            </span>
            {typing ? (
              <span className="-my-0.5 -mr-0.5 grid size-5 flex-none place-items-center self-end rounded-full bg-(--chatgpt-ink) text-(--chatgpt-main)">
                <ArrowUp aria-hidden className="size-2.5" strokeWidth={2.5} />
              </span>
            ) : (
              <Mic
                aria-hidden
                className="mt-0.5 size-3 flex-none text-(--chatgpt-muted)"
              />
            )}
          </div>
          <NeoConnectedChip
            className={cn(
              'border-(--chatgpt-border) bg-(--chatgpt-sidebar) text-(--chatgpt-text)',
              !work && 'invisible opacity-0',
            )}
          />
        </div>
      </div>
    </div>
  )
}
