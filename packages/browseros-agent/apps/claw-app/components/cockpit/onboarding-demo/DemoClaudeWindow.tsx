/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { CLAUDE_LIGHT } from './claude-light'
import {
  Caret,
  DemoToolRows,
  NeoConnectedChip,
  TrafficLights,
  Transcript,
} from './demo-parts'
import { DEMO_PROMPT, type DemoFrame } from './timeline'

/** Chat and Code keep different histories; the Switch beat changes both. */
const RECENTS = {
  chat: ['Weekend trip ideas', 'Thank-you note draft'],
  code: ['Fix the signup bug', 'Sort my inbox'],
} as const

const ROW = 'flex h-7 flex-none items-center gap-[7px] rounded-[7px] px-2'

/**
 * The Claude desktop app, the way most people run Claude Code (owner, round
 * 5), in Claude's own default light theme (round 8) and the system face, as
 * Paper draws it. It opens on Chat, and the Switch beat moves it to Code
 * (round 9): only then do the code recents, the code placeholder and the
 * "BrowserOS neo ✓" chip (the one sign of the connection) appear. The cockpit
 * window has no "Browsing…" line: tool row ③ already shows the work (Paper).
 */
export function DemoClaudeWindow({
  frame,
  front,
}: {
  frame: DemoFrame['agent']
  front: boolean
}) {
  const typing = frame.composer.length > 0
  const code = frame.switched
  const blank = code ? 'New session' : 'New chat'
  return (
    <div
      style={CLAUDE_LIGHT}
      className="flex size-full overflow-hidden rounded-xl bg-(--claude-main) font-[system-ui,sans-serif] shadow-[0_0_0_1px_rgb(0_0_0/0.1),0_34px_60px_-28px_rgb(0_0_0/0.38),0_10px_24px_-12px_rgb(0_0_0/0.18)]"
    >
      <div className="flex w-[146px] flex-none flex-col gap-1 border-(--claude-border) border-r bg-(--claude-sidebar) px-2.5 pb-3">
        <div className="flex h-9 flex-none items-center gap-1.5 pl-0.5">
          <TrafficLights size={10} />
          <span className="w-1.5" />
          <svg aria-hidden width="15" height="15" viewBox="0 0 16 16">
            <rect
              x="2"
              y="3"
              width="12"
              height="10"
              rx="2"
              fill="none"
              stroke="#717783"
              strokeWidth="1.3"
            />
            <path d="M6 3v10" stroke="#717783" strokeWidth="1.3" />
          </svg>
          <svg aria-hidden width="14" height="14" viewBox="0 0 16 16">
            <path
              d="M13 8H3M7 4L3 8l4 4"
              fill="none"
              stroke="#717783"
              strokeWidth="1.4"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
          <svg aria-hidden width="14" height="14" viewBox="0 0 16 16">
            <path
              d="M3 8h10M9 4l4 4-4 4"
              fill="none"
              stroke="#C3CAD6"
              strokeWidth="1.4"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </div>
        {/* Chat | Code: the white segment moves to Code on the Switch beat. */}
        <div className="mb-1.5 flex gap-0.5 rounded-lg bg-(--claude-toggle) p-0.5">
          <ToggleSegment on={!code} label="Chat">
            <path
              d="M3 3.5h10v7H7l-3 2.5v-2.5H3z"
              fill="none"
              strokeWidth="1.4"
              strokeLinejoin="round"
            />
          </ToggleSegment>
          <ToggleSegment on={code} label="Code">
            <path
              d="M6 4.5L2.5 8 6 11.5M10 4.5L13.5 8 10 11.5"
              fill="none"
              strokeWidth="1.6"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </ToggleSegment>
        </div>
        <span className="h-1 flex-none" />
        <div
          className={cn(
            ROW,
            'font-medium text-(--claude-strong) text-[12px] leading-4 transition-colors duration-300',
            !frame.sent && 'bg-(--claude-selected)',
          )}
        >
          <svg aria-hidden width="12" height="12" viewBox="0 0 16 16">
            <path
              d="M8 3v10M3 8h10"
              fill="none"
              stroke="#575D69"
              strokeWidth="1.6"
              strokeLinecap="round"
            />
          </svg>
          {blank}
        </div>
        <div className="px-2 pt-2.5 pb-1 text-(--claude-muted) text-[11px] leading-[14px]">
          Recents
        </div>
        {frame.sent && (
          <div
            className={cn(
              ROW,
              'animate-demo-rise bg-(--claude-selected) font-medium text-(--claude-ink) text-[12px] leading-4',
            )}
          >
            <span className="min-w-0 flex-1 truncate">Cheapest flight</span>
            <i
              className={cn(
                'size-1.5 flex-none rounded-full bg-(--claude-orange)',
                frame.working && 'animate-pulse-dot',
              )}
            />
          </div>
        )}
        {RECENTS[code ? 'code' : 'chat'].map((title) => (
          <div
            key={title}
            className={cn(ROW, 'text-(--claude-text) text-[12px] leading-4')}
          >
            <span className="min-w-0 truncate">{title}</span>
          </div>
        ))}
      </div>

      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-9 flex-none items-center px-4 font-medium text-(--claude-text) text-[12px] leading-4">
          {frame.sent ? 'Cheapest flight' : blank}
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-2 px-4 pt-0.5 pb-2.5">
          <Transcript gap={8}>
            {frame.sent && (
              <div className="animate-demo-rise rounded-[10px] bg-(--claude-bubble) px-3 py-2 text-(--claude-ink) text-[13px] leading-[18px]">
                {DEMO_PROMPT}
              </div>
            )}
            <DemoToolRows
              shown={frame.tools}
              lit={frame.litTool}
              tone="claude"
            />
            {frame.answered && (
              <div className="flex animate-demo-rise items-start gap-[9px] pt-0.5 pr-2 pl-[9px] font-medium text-(--claude-ink) text-[13.5px] leading-5">
                <span className="mt-px grid size-[18px] flex-none place-items-center rounded-full bg-(--claude-text)">
                  <svg aria-hidden width="11" height="11" viewBox="0 0 16 16">
                    <path
                      d="M3.5 8.5l3 3 6-7"
                      fill="none"
                      stroke="#fff"
                      strokeWidth="2.2"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                </span>
                Cheapest is $142, nonstop, Fri 7:05 am.
              </div>
            )}
          </Transcript>
          <div className="flex flex-none flex-col gap-[7px]">
            <div
              className={cn(
                'flex flex-col gap-1 rounded-xl border bg-white pt-2 pr-[9px] pb-1.5 pl-3 text-[13px] leading-[19px] transition-[border-color,box-shadow] duration-200',
                typing
                  ? 'border-[#BEBDB9] shadow-[0_0_0_3px_rgb(0_0_0/0.035),0_1px_3px_rgb(0_0_0/0.06)]'
                  : 'border-(--claude-composer-border) shadow-[0_1px_3px_rgb(0_0_0/0.05)]',
              )}
            >
              {typing ? (
                <span className="text-(--claude-ink)">
                  {frame.composer}
                  {front && <Caret className="ml-px h-4" />}
                </span>
              ) : (
                <span className="text-(--claude-muted)">
                  {code
                    ? 'Describe a task or ask a question'
                    : 'How can I help you today?'}
                </span>
              )}
              <div className="flex items-center justify-between">
                <svg aria-hidden width="14" height="14" viewBox="0 0 16 16">
                  <path
                    d="M8 3v10M3 8h10"
                    fill="none"
                    stroke="#717783"
                    strokeWidth="1.6"
                    strokeLinecap="round"
                  />
                </svg>
                <span
                  className={cn(
                    'grid size-6 place-items-center rounded-full bg-(--claude-orange) transition-opacity duration-200',
                    !typing && 'opacity-45',
                  )}
                >
                  <svg aria-hidden width="12" height="12" viewBox="0 0 16 16">
                    <path
                      d="M8 13V3M4 7l4-4 4 4"
                      fill="none"
                      stroke="#fff"
                      strokeWidth="1.8"
                      strokeLinecap="round"
                      strokeLinejoin="round"
                    />
                  </svg>
                </span>
              </div>
            </div>
            <NeoConnectedChip
              className={cn(
                'bg-(--claude-bubble) text-(--claude-text)',
                !code && 'invisible',
              )}
            />
          </div>
        </div>
      </div>
    </div>
  )
}

/** One half of Chat | Code: the selected half is white and bold. */
function ToggleSegment({
  on,
  label,
  children,
}: {
  on: boolean
  label: string
  children: ReactNode
}) {
  return (
    <span
      className={cn(
        'flex h-6 flex-1 items-center justify-center gap-[5px] rounded-md text-[12px] leading-4 transition-[background-color,color,box-shadow] duration-300',
        on
          ? 'bg-white font-semibold text-(--claude-strong) shadow-[0_0_0_1px_rgb(0_0_0/0.06),0_1px_2px_rgb(0_0_0/0.1)]'
          : 'font-medium text-[#898881]',
      )}
    >
      <svg
        aria-hidden
        width="12"
        height="12"
        viewBox="0 0 16 16"
        className={cn(
          'transition-[stroke] duration-300',
          on ? 'stroke-(--claude-strong)' : 'stroke-[#717783]',
        )}
      >
        {children}
      </svg>
      {label}
    </span>
  )
}
