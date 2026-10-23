/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { Caret, DemoNeoMark, StepBadge, TrafficLights } from './demo-parts'
import { DEMO_CHEAPEST_ROW, DEMO_FLIGHTS, type DemoFrame } from './timeline'

const ROW_HEIGHT = 36

/** The agent's tag on neo's tab: its name after an amber dot. */
function AgentTag({
  state,
  agent,
}: {
  state: DemoFrame['neo']['agentTag']
  agent: string
}) {
  return (
    <span
      className={cn(
        'flex h-[19px] flex-none items-center gap-[5px] rounded-[10px] bg-[#EAF1FF] px-2 font-medium font-mono text-[#0254EC] text-[11px] leading-3 transition-opacity duration-300',
        state === 'off' && 'opacity-0',
      )}
    >
      <i
        className={cn(
          'size-1.5 rounded-full bg-[#F5B957]',
          state === 'working' && 'animate-pulse-dot',
        )}
      />
      {agent}
    </span>
  )
}

function SearchField({
  label,
  value,
  lit,
  typed,
  chip,
}: {
  label: string
  value: string
  lit?: boolean
  /** The agent fills this field, so it turns blue once filled. */
  typed?: boolean
  chip?: ReactNode
}) {
  const set = typed && value.length > 0
  return (
    <div
      className={cn(
        'flex flex-1 flex-col gap-0.5 rounded-[10px] border px-3 py-1.5 transition-[border-color,box-shadow,background-color] duration-300',
        lit
          ? 'border-[#0254EC] bg-[#F4F8FF] shadow-[0_0_0_3px_rgb(2_84_236/0.14)]'
          : set
            ? 'border-[#9DBBF5] bg-[#F4F8FF]'
            : 'border-[#DCE4F2] bg-white',
      )}
    >
      <span className="flex h-[13px] items-center gap-1.5 font-mono text-[#717783] text-[11px] leading-[13px] tracking-[0.1em]">
        {label}
        {chip}
      </span>
      <span
        className={cn(
          'flex h-5 items-center whitespace-nowrap text-[15px] leading-5',
          set ? 'font-semibold' : 'font-medium',
        )}
      >
        {value}
        {lit && <Caret className="ml-px h-4 text-[#0254EC]" />}
      </span>
    </div>
  )
}

/**
 * neo's window, the same flight scene in every flow: a new tab waits beside
 * the agent's app, then Google Flights opens, fills in and is read row by row.
 * The numbered badges mark what each of the agent's tool rows touched.
 */
export function DemoNeoWindow({
  frame,
  agent,
}: {
  frame: DemoFrame['neo']
  /** The agent's name on neo's tab tag: "claude", or "codex" in ChatGPT. */
  agent: string
}) {
  const typed = frame.url.length > 0
  return (
    <div className="flex size-full flex-col overflow-hidden rounded-xl bg-white text-[#01123F] shadow-[0_0_0_1px_rgb(1_18_63/0.1),0_40px_70px_-30px_rgb(1_18_63/0.45),0_10px_24px_-12px_rgb(1_18_63/0.2)]">
      <div className="flex h-[38px] flex-none items-end gap-1.5 bg-[#EAF1FF] pr-3 pl-3.5">
        <span className="self-center pr-2.5">
          <TrafficLights size={11} />
        </span>
        <div className="flex h-[31px] w-60 flex-none items-center gap-[9px] rounded-t-[9px] bg-white pr-[9px] pl-3">
          <i
            className={cn(
              'size-[13px] flex-none rounded-[3px] transition-colors duration-300',
              frame.pageShown ? 'bg-[#01123F]' : 'bg-[#D3E2FF]',
            )}
          />
          <span className="min-w-0 flex-1 truncate font-medium text-[13px] leading-4">
            {frame.pageShown ? 'Flights' : 'New Tab'}
          </span>
          <AgentTag agent={agent} state={frame.agentTag} />
        </div>
        <span className="self-center px-1.5 text-[#717783] text-[17px] leading-[18px]">
          +
        </span>
      </div>

      <div className="flex h-11 flex-none items-center gap-3 border-[#EEF2F8] border-b px-3.5">
        <svg aria-hidden width="58" height="16" viewBox="0 0 58 16">
          <path
            d="M9 3L4 8l5 5M4 8h9"
            fill="none"
            stroke="#575D69"
            strokeWidth="1.6"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          <path
            d="M31 3l5 5-5 5M36 8h-9"
            fill="none"
            stroke="#B8BFCC"
            strokeWidth="1.6"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          <path
            d="M56 8a5 5 0 1 1-1.6-3.7M56 2.5v2.8h-2.8"
            fill="none"
            stroke="#575D69"
            strokeWidth="1.6"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        </svg>
        <div
          className={cn(
            'relative flex h-8 min-w-0 flex-1 items-center gap-2 rounded-2xl border bg-[#F6F8FC] pr-10 pl-3.5 transition-[border-color,box-shadow] duration-300',
            frame.urlLit
              ? 'border-[#9DBBF5] shadow-[0_0_0_3px_rgb(2_84_236/0.12)]'
              : 'border-[#CBD5E1]/60',
          )}
        >
          {typed ? (
            <>
              <svg aria-hidden width="11" height="13" viewBox="0 0 11 13">
                <rect
                  x="1"
                  y="5.5"
                  width="9"
                  height="6.5"
                  rx="1.6"
                  fill="#717783"
                />
                <path
                  d="M3 5.5V4a2.5 2.5 0 0 1 5 0v1.5"
                  fill="none"
                  stroke="#717783"
                  strokeWidth="1.4"
                />
              </svg>
              <span className="whitespace-nowrap font-mono text-[13px] leading-4">
                {frame.url}
              </span>
            </>
          ) : (
            <span className="whitespace-nowrap text-[#9AA1AE] text-[13.5px] leading-4">
              Search or type a URL
            </span>
          )}
          <StepBadge
            n={1}
            state={frame.badges[0]}
            className="absolute top-[5px] right-[5px]"
          />
          <i
            className={cn(
              'absolute inset-x-3.5 -bottom-px h-0.5 origin-left rounded-[1px] bg-[#0254EC]',
              frame.loading === 'loading'
                ? 'transition-transform duration-500 ease-out'
                : 'opacity-0',
              frame.loading === 'idle' && 'scale-x-0',
            )}
          />
        </div>
        <DemoNeoMark className="size-[22px] rounded-md" />
      </div>

      {frame.pageShown ? (
        <div className="flex flex-1 animate-demo-rise flex-col gap-3 px-6 pt-3">
          <div className="relative flex gap-2">
            <SearchField label="FROM" value="San Francisco" />
            <SearchField label="TO" value={frame.to} lit={frame.toLit} typed />
            <SearchField
              label="WHEN"
              value={frame.when}
              lit={frame.whenLit}
              typed
              chip={
                <span className="rounded-[4px] bg-[#EAF1FF] px-[5px] text-[#0254EC] tracking-normal">
                  One way
                </span>
              }
            />
            <StepBadge
              n={2}
              state={frame.badges[1]}
              className="absolute -top-[9px] -right-[9px]"
            />
          </div>

          <div
            className={cn(
              'relative flex flex-col transition-opacity duration-400 ease-out',
              frame.results ? 'opacity-100' : 'opacity-0',
            )}
          >
            <div className="flex items-center justify-between pb-1.5">
              <b className="font-bold text-[17px] leading-[22px]">
                To New York · Fri
              </b>
              <span className="mr-[18px] flex h-6 items-center rounded-xl bg-[#EAF1FF] px-2.5 font-medium font-mono text-[#0254EC] text-[11.5px] leading-[14px]">
                Checked {frame.checked}/5
              </span>
            </div>
            <StepBadge
              n={3}
              state={frame.badges[2]}
              className="absolute top-px -right-[9px]"
            />
            <div className="relative flex flex-col">
              {/* The reading ring slides row to row behind the text, then
                  settles on the answer at Done. */}
              <div
                aria-hidden
                style={{ top: (frame.ring ?? 0) * ROW_HEIGHT }}
                className={cn(
                  'pointer-events-none absolute inset-x-0 h-9 rounded-[10px] transition-[top,opacity,background-color,box-shadow] duration-500 ease-out',
                  frame.ring === null ? 'opacity-0' : 'opacity-100',
                  frame.ringSettled
                    ? 'bg-[#F4F8FF] shadow-[0_0_0_1.5px_#9DBBF5]'
                    : 'bg-white shadow-[0_0_0_2px_#0254EC,0_0_0_6px_rgb(2_84_236/0.14)]',
                )}
              />
              {DEMO_FLIGHTS.map((flight, index) => {
                const besideRing =
                  frame.ring === index || frame.ring === index + 1
                return (
                  <div
                    key={flight.time}
                    className={cn(
                      'relative flex h-9 flex-none items-center gap-3.5 border-b px-3.5',
                      besideRing || index === DEMO_FLIGHTS.length - 1
                        ? 'border-transparent'
                        : 'border-[#EEF2F8]',
                    )}
                  >
                    <span className="w-[84px] flex-none font-semibold text-[15px] leading-5">
                      {flight.time}
                    </span>
                    <span className="w-20 flex-none text-[#717783] text-[14px] leading-[18px]">
                      {flight.stops}
                    </span>
                    <i className="h-1.5 w-12 flex-none rounded-[3px] bg-[#E6ECF6]" />
                    <span className="flex-1" />
                    <span className="flex w-24 flex-none justify-end">
                      {index === DEMO_CHEAPEST_ROW && (
                        <span
                          className={cn(
                            'flex h-6 items-center rounded-xl bg-[#0254EC] px-[11px] font-mono font-semibold text-[11.5px] text-white leading-[14px] transition-[opacity,transform] duration-400 ease-out',
                            frame.tagged ? 'opacity-100' : 'scale-85 opacity-0',
                          )}
                        >
                          Cheapest
                        </span>
                      )}
                    </span>
                    <span className="w-14 flex-none text-right font-bold text-[16px] leading-5">
                      {flight.price}
                    </span>
                  </div>
                )
              })}
            </div>
          </div>
        </div>
      ) : (
        <div className="flex flex-1 items-center justify-center px-7 pt-5 pb-10">
          <DemoNeoMark className="size-[72px] rounded-[18px] opacity-18" />
        </div>
      )}
    </div>
  )
}
