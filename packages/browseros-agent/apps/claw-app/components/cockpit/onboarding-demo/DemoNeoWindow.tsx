/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Check, ChevronLeft, ChevronRight, Lock, RotateCw } from 'lucide-react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { Caret, DemoNeoMark, StepBadge, TrafficLights } from './demo-parts'
import { DEMO_CHEAPEST_ROW, DEMO_FLIGHTS, type DemoFrame } from './timeline'

const ROW_HEIGHT = 33

/** The agent's tag: amber while it works, a tick once Claude has answered. */
function AgentTag({
  state,
  agent,
  className,
}: {
  state: DemoFrame['neo']['agentTag']
  agent: string
  className?: string
}) {
  return (
    <span
      className={cn(
        'inline-flex h-[19px] items-center gap-[5px] rounded-[10px] bg-accent-tint pr-[7px] pl-1.5 font-mono font-semibold text-[11px] text-accent transition-opacity duration-300',
        className,
      )}
    >
      {state === 'done' ? null : (
        <i
          className={cn(
            'size-1.5 rounded-full',
            state === 'working'
              ? 'animate-pulse-dot bg-chart-3'
              : 'bg-ink-4 opacity-60',
          )}
        />
      )}
      {agent}
      {state === 'done' && (
        <Check aria-hidden className="size-2.5" strokeWidth={3} />
      )}
    </span>
  )
}

function SearchField({
  label,
  value,
  lit,
  badge,
}: {
  label: string
  value: string
  lit?: boolean
  badge?: ReactNode
}) {
  return (
    <div
      className={cn(
        'relative flex h-[46px] flex-col justify-center gap-px rounded-[9px] border-[1.5px] px-[11px] transition-[border-color,box-shadow,background-color] duration-300',
        lit
          ? 'border-accent bg-bg-canvas shadow-[0_0_0_3px_rgb(2_84_236/0.14)]'
          : 'border-border bg-card',
      )}
    >
      <span className="font-medium font-mono text-[11px] text-ink-3 uppercase leading-3 tracking-[0.1em]">
        {label}
      </span>
      <span className="flex h-[19px] items-center whitespace-nowrap font-medium text-[14.5px]">
        {value}
        {lit && <Caret className="ml-px h-4 text-accent" />}
      </span>
      {badge}
    </div>
  )
}

/** neo waits behind on its new tab, then comes forward and does the chore. */
export function DemoNeoWindow({
  frame,
  fading,
  agent,
}: {
  frame: DemoFrame['neo']
  fading: boolean
  /** The agent's name on neo's tab tag: "claude", or "codex" in ChatGPT. */
  agent: string
}) {
  const loaded = frame.loading === 'loaded'
  return (
    <div
      className={cn(
        'absolute top-3 left-[456px] flex h-[378px] w-[480px] origin-top flex-col overflow-hidden rounded-xl border border-border bg-card transition-[transform,box-shadow,opacity] duration-500 ease-out',
        frame.up
          ? 'z-5 shadow-[0_44px_80px_-36px_rgb(1_18_63/0.5),0_6px_18px_-6px_rgb(1_18_63/0.16)]'
          : 'z-2 translate-y-2 scale-[0.97] shadow-[0_24px_48px_-30px_rgb(1_18_63/0.32)]',
        fading && 'opacity-0',
      )}
    >
      <div className="flex h-9 flex-none items-end gap-2.5 border-border-2 border-b bg-card-tint px-3">
        <span className="self-center">
          <TrafficLights active={frame.up} />
        </span>
        <div className="flex h-[29px] w-[236px] items-center gap-2 rounded-t-lg border border-border-2 border-b-0 bg-card pr-2 pl-[11px] font-medium text-[12.5px]">
          {loaded ? (
            <i className="size-[15px] flex-none rounded-[4px] bg-ink-deep" />
          ) : (
            <DemoNeoMark className="size-[15px] rounded-[4px]" />
          )}
          <span className="min-w-0 flex-1 truncate">
            {loaded ? 'Flights' : 'New Tab'}
          </span>
          <AgentTag
            agent={agent}
            state={frame.agentTag}
            className={frame.agentTag === 'off' ? 'opacity-0' : ''}
          />
        </div>
        <span className="self-center px-1 text-base text-ink-3">+</span>
      </div>

      <div className="flex h-[42px] flex-none items-center gap-1 border-accent-tint border-b px-3 text-ink-3">
        <ChevronLeft aria-hidden className="size-[15px]" />
        <ChevronRight aria-hidden className="size-[15px]" />
        <RotateCw aria-hidden className="size-[13px]" />
        <div
          className={cn(
            'relative ml-1.5 flex h-7 min-w-0 flex-1 items-center gap-[7px] rounded-lg border-[1.5px] pr-8 pl-[11px] font-mono text-[12px] text-cyanotype-ink leading-4 transition-[border-color,box-shadow,background-color] duration-300',
            frame.urlLit
              ? 'border-accent bg-card shadow-[0_0_0_3px_rgb(2_84_236/0.14)]'
              : 'border-transparent bg-card-tint',
          )}
        >
          {loaded && <Lock aria-hidden className="size-[11px] text-ink-3" />}
          {frame.url ? (
            <span className="whitespace-nowrap">{frame.url}</span>
          ) : (
            <span className="whitespace-nowrap text-ink-4">
              Search or type a URL
            </span>
          )}
          <StepBadge
            n={1}
            state={frame.badges[0]}
            className="absolute top-[2.5px] right-[3px]"
          />
        </div>
        <DemoNeoMark className="ml-2 size-5" />
      </div>

      <div className="relative flex-1 overflow-hidden">
        <div
          className={cn(
            'absolute top-0 left-0 z-3 h-0.5 w-full origin-left bg-accent',
            frame.loading === 'idle' && 'scale-x-0',
            frame.loading === 'loading' &&
              'scale-x-80 transition-transform duration-500 ease-out',
            loaded &&
              'opacity-0 transition-[transform,opacity] delay-150 duration-300',
          )}
        />
        <div
          className={cn(
            'absolute inset-0 flex flex-col items-center gap-3.5 bg-bg-canvas pt-12 transition-[opacity,transform] duration-300 ease-out',
            frame.pageShown && '-translate-y-1.5 opacity-0',
          )}
        >
          <DemoNeoMark className="size-9" />
          <div className="text-center font-bold text-[19px] leading-tight tracking-[-0.02em]">
            What are your agents
            <br />
            <em className="font-medium font-serif">working on</em> right now?
          </div>
          <div className="flex items-center gap-2.5 rounded-xl border border-border-2 bg-card px-[13px] py-[9px] text-[12.5px] text-ink-3 shadow-[0_6px_16px_-10px_rgb(1_18_63/0.2)]">
            <AgentTag
              agent={agent}
              state={frame.agentTag === 'off' ? 'off' : 'working'}
            />
            {frame.agentTag === 'off' ? 'Waiting for a task' : 'Starting…'}
          </div>
        </div>

        <div
          className={cn(
            'absolute inset-0 px-[18px] pt-3 transition-opacity duration-300',
            frame.pageShown ? 'opacity-100' : 'opacity-0',
          )}
        >
          <div className="grid grid-cols-3 gap-2">
            <SearchField label="From" value="San Francisco" />
            <SearchField label="To" value={frame.to} lit={frame.toLit} />
            <SearchField
              label="When"
              value={frame.when}
              lit={frame.whenLit}
              badge={
                <StepBadge
                  n={2}
                  state={frame.badges[1]}
                  className="absolute -top-[9px] -right-[9px]"
                />
              }
            />
          </div>

          <div
            className={cn(
              'relative mt-2.5 rounded-[10px] px-1.5 pt-0.5 transition-shadow duration-300',
              frame.resultsLit && 'shadow-[0_0_0_1.5px_rgb(2_84_236/0.5)]',
            )}
          >
            <StepBadge
              n={3}
              state={frame.badges[2]}
              className="absolute -top-[9px] -right-[9px]"
            />
            <div
              className={cn(
                'flex h-[30px] items-center justify-between transition-opacity duration-300',
                frame.results ? 'opacity-100' : 'opacity-0',
              )}
            >
              <b className="font-bold text-base">To New York · Fri</b>
              <span className="flex h-[22px] items-center gap-[5px] rounded-[11px] bg-accent-tint px-[9px] font-medium font-mono text-[11px] text-accent leading-none">
                Checked <b className="font-bold">{frame.checked}/5</b>
              </span>
            </div>
            <div className="relative">
              {DEMO_FLIGHTS.map((flight, index) => (
                <div
                  key={flight.time}
                  style={{ transitionDelay: `${index * 50}ms` }}
                  className={cn(
                    'flex h-[33px] items-center gap-3 border-accent-tint border-b px-2 transition-[opacity,transform] duration-300 ease-out last:border-b-0',
                    frame.results ? 'opacity-100' : 'translate-y-1 opacity-0',
                  )}
                >
                  <span className="w-16 flex-none font-semibold text-[14.5px]">
                    {flight.time}
                  </span>
                  <span className="w-[62px] flex-none text-[13.5px] text-ink-3">
                    {flight.stops}
                  </span>
                  <i className="h-[7px] flex-1 rounded-[4px] bg-accent-tint" />
                  <span className="flex w-[76px] flex-none justify-end">
                    {index === DEMO_CHEAPEST_ROW && (
                      <span
                        className={cn(
                          'flex h-[22px] items-center rounded-[11px] bg-accent px-[9px] font-mono font-semibold text-[11px] text-accent-foreground leading-none transition-opacity duration-400',
                          frame.tagged ? 'opacity-100' : 'opacity-0',
                        )}
                      >
                        Cheapest
                      </span>
                    )}
                  </span>
                  <span className="w-[46px] flex-none text-right font-bold text-[15px]">
                    {flight.price}
                  </span>
                </div>
              ))}
              <div
                aria-hidden
                style={{ top: 1 + (frame.ring ?? 0) * ROW_HEIGHT }}
                className={cn(
                  'pointer-events-none absolute -inset-x-1 h-[31px] rounded-[10px] transition-[top,opacity] duration-500 ease-out',
                  frame.ring === null ? 'opacity-0' : 'opacity-100',
                  frame.ringSettled
                    ? 'border-[1.5px] border-accent/35 bg-accent/5'
                    : 'border-2 border-accent shadow-[0_0_0_4px_rgb(2_84_236/0.12)]',
                )}
              />
            </div>
          </div>
        </div>
      </div>
    </div>
  )
}
