/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { cn } from '@/lib/utils'
import { CHATGPT_DARK } from './chatgpt-dark'
import {
  Caret,
  DemoToolRows,
  NeoConnectedChip,
  TrafficLights,
  Transcript,
} from './demo-parts'
import { DEMO_PROMPT, type DemoFrame } from './timeline'

/** ChatGPT keeps one history across Chat and Work (Paper). */
const RECENTS = ['Weekend trip ideas', 'Thank-you note draft'] as const

const ROW =
  'flex h-[30px] flex-none items-center rounded-lg px-1.5 text-[13px] leading-[18px]'

/**
 * The ChatGPT desktop app in its dark theme, drawn from the owner's
 * screenshot (round 10): Codex runs inside it, behind the Work half of the
 * Chat | Work toggle at the top. It opens on Chat, with its greeting and
 * composer centred; the Switch beat moves it to Work. Paper draws this window
 * only at the 1200 px beat size; this is that design fitted to the cockpit's
 * grid, so the cockpit window, like Claude's, has no "Working…" line.
 */
export function DemoChatGptWindow({
  frame,
  front,
}: {
  frame: DemoFrame['agent']
  front: boolean
}) {
  const typing = frame.composer.length > 0
  const work = frame.switched
  return (
    <div
      style={CHATGPT_DARK}
      className="flex size-full overflow-hidden rounded-xl bg-(--chatgpt-main) text-(--chatgpt-ink) shadow-[0_0_0_1px_rgb(0_0_0/0.35),0_34px_60px_-28px_rgb(1_18_63/0.55),0_10px_24px_-12px_rgb(1_18_63/0.3)]"
    >
      <div className="flex w-[140px] flex-none flex-col gap-0.5 border-(--chatgpt-border) border-r bg-(--chatgpt-sidebar) px-2.5 pb-3">
        <div className="flex h-9 flex-none items-center pl-0.5">
          <TrafficLights size={10} />
        </div>
        <div className="flex h-[30px] flex-none items-center gap-[5px] px-1.5 font-semibold text-[15px] leading-5">
          ChatGPT
          <svg aria-hidden width="10" height="10" viewBox="0 0 16 16">
            <path
              d="M4 6l4 4 4-4"
              fill="none"
              stroke="#8C8D90"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </div>
        <div
          className={cn(
            ROW,
            'gap-2 text-(--chatgpt-text) transition-colors duration-300',
            !frame.sent && 'bg-(--chatgpt-selected)',
          )}
        >
          <svg aria-hidden width="13" height="13" viewBox="0 0 16 16">
            <path
              d="M11.5 2.5l2 2L7 11H5V9l6.5-6.5zM3 13.5h10"
              fill="none"
              stroke="#DADAD6"
              strokeWidth="1.4"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
          New chat
        </div>
        <div className="px-1.5 pt-3 pb-1 text-(--chatgpt-muted) text-[12px] leading-4">
          Recents
        </div>
        {frame.sent && (
          <div className={cn(ROW, 'animate-demo-rise bg-(--chatgpt-selected)')}>
            Cheapest flight
          </div>
        )}
        {RECENTS.map((title) => (
          <div key={title} className={cn(ROW, 'text-(--chatgpt-text)')}>
            <span className="min-w-0 truncate">{title}</span>
          </div>
        ))}
      </div>

      <div className="flex min-w-0 flex-1 flex-col">
        {/* Chat | Work, centred at the top of the conversation. */}
        <div className="flex h-9 flex-none items-center justify-center">
          <div className="flex gap-0.5 rounded-full bg-(--chatgpt-toggle) p-[3px]">
            {(['Chat', 'Work'] as const).map((label) => {
              const on = (label === 'Work') === work
              return (
                <span
                  key={label}
                  className={cn(
                    'flex h-6 items-center rounded-full px-4 font-medium text-[12.5px] leading-4 transition-[background-color,color,box-shadow] duration-300',
                    on
                      ? 'bg-(--chatgpt-toggle-on) text-(--chatgpt-ink) shadow-[0_0_0_1px_#4B4C55]'
                      : 'text-(--chatgpt-soft)',
                  )}
                >
                  {label}
                </span>
              )
            })}
          </div>
        </div>
        <div
          className={cn(
            'flex min-h-0 flex-1 flex-col px-4 pt-1 pb-3',
            frame.sent ? 'gap-1.5' : 'gap-4',
          )}
        >
          {frame.sent ? (
            <Transcript gap={6}>
              <div className="max-w-[236px] animate-demo-rise self-end rounded-2xl bg-(--chatgpt-selected) px-[13px] py-2 text-[13px] leading-[18px]">
                {DEMO_PROMPT}
              </div>
              <DemoToolRows
                shown={frame.tools}
                lit={frame.litTool}
                tone="chatgpt"
              />
              {frame.answered && (
                <div className="flex animate-demo-rise items-start gap-[9px] pt-0.5 pr-2 pl-[9px] font-medium text-[13.5px] leading-5">
                  <span className="mt-px grid size-[18px] flex-none place-items-center rounded-full bg-(--chatgpt-ink)">
                    <svg aria-hidden width="11" height="11" viewBox="0 0 16 16">
                      <path
                        d="M3.5 8.5l3 3 6-7"
                        fill="none"
                        stroke="#292A36"
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
          ) : (
            <>
              <span className="flex-1" />
              <div className="text-center text-[22px] leading-7">
                {work ? 'What should we work on?' : 'What can I help with?'}
              </div>
            </>
          )}
          <div className="flex flex-none flex-col gap-[7px]">
            <div className="flex flex-col gap-2 rounded-[18px] bg-(--chatgpt-composer) pt-2.5 pr-2.5 pb-2 pl-3.5 text-[13px] leading-[18px]">
              {typing ? (
                <span>
                  {frame.composer}
                  {front && <Caret className="ml-px h-4" />}
                </span>
              ) : (
                <span className="text-(--chatgpt-muted-2)">Ask anything</span>
              )}
              <div className="flex items-center justify-between">
                <svg aria-hidden width="14" height="14" viewBox="0 0 16 16">
                  <path
                    d="M8 3v10M3 8h10"
                    fill="none"
                    stroke="#DADAD6"
                    strokeWidth="1.6"
                    strokeLinecap="round"
                  />
                </svg>
                <span
                  className={cn(
                    'grid size-6 place-items-center rounded-full transition-colors duration-200',
                    typing ? 'bg-(--chatgpt-ink)' : 'bg-(--chatgpt-ink)/40',
                  )}
                >
                  <svg aria-hidden width="12" height="12" viewBox="0 0 16 16">
                    <path
                      d="M8 13V3M4 7l4-4 4 4"
                      fill="none"
                      stroke="#292A36"
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
                'bg-(--chatgpt-toggle) text-(--chatgpt-text)',
                !work && 'invisible',
              )}
            />
          </div>
          {!frame.sent && <span className="flex-1" />}
        </div>
      </div>
    </div>
  )
}
