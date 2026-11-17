/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Home for an ultrafast customer: the total tokens saved since this computer got ultrafast, read
 * off a ruled log scale with one needle, and a short ledger beside it. Free installs keep SavingsCard.
 */

import type {
  CockpitStats,
  PlanSavings,
  PlanSavingsMeter,
} from '@browseros/claw-api'
import type { UseQueryResult } from '@tanstack/react-query'
import { ArrowRight, Copy, Info } from 'lucide-react'
import { type ReactNode, useEffect, useState } from 'react'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/motion/tooltip'
import { cn } from '@/lib/utils'
import { ultrafastTime } from '@/screens/audit/audit.helpers'
import { roundTimeSaved } from '@/screens/cockpit/cockpit.helpers'
import {
  HOW_WE_COUNT,
  lastReading,
  meterNote,
  meterTokens,
  nextMilestone,
  rememberReading,
  scaleMarks,
  scalePosition,
  scaleTicks,
  scaleTop,
  tokensSavedSinceUltrafast,
} from './energy-meter.helpers'
import { BigNumber, INK, PANEL_SHADOW, Wordmark } from './InkPanel'
import { timeSavedAmount } from './savings.helpers'
import { UltrafastDetailsDialog } from './UltrafastDetailsDialog'
import { monthDay } from './ultrafast-details.helpers'

export const TRY_TASK =
  'On google.com/travel/flights, search one-way flights from Chennai to Singapore on 20 December and sort by price.'

const LINK =
  'inline-flex h-8 items-center gap-1.5 rounded-lg px-2 -mx-2 text-[13.5px] hover:bg-[rgb(238_243_255/0.07)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#9cc0ff] active:translate-y-px'

export function EnergyMeter({
  savings,
  stats,
  className,
}: {
  savings: UseQueryResult<PlanSavings>
  stats: CockpitStats | undefined
  className?: string
}) {
  if (savings.isPending) return <Loading className={className} />
  const meter = savings.data?.meter
  const tokens = savings.data ? tokensSavedSinceUltrafast(savings.data) : null
  if (savings.isError || !meter || tokens === null) {
    return (
      <Panel className={className}>
        <div className="flex flex-col items-start gap-3">
          <Wordmark />
          <p role="alert" className={cn('m-0 text-[14.5px]', INK.ink2)}>
            Your ultrafast numbers didn't load.
          </p>
          <button
            type="button"
            onClick={() => savings.refetch()}
            className={cn(
              'h-[34px] rounded-lg border px-3.5 font-semibold text-[13.5px] hover:bg-[rgb(238_243_255/0.07)] active:translate-y-px',
              INK.line,
              INK.ink,
            )}
          >
            Try again
          </button>
        </div>
      </Panel>
    )
  }
  const plan = savings.data
  if (plan.tasks === 0) {
    return (
      <FirstRun
        savings={plan}
        tokens={tokens}
        stats={stats}
        className={className}
      />
    )
  }
  return (
    <Meter
      savings={plan}
      meter={meter}
      tokens={tokens}
      stats={stats}
      className={className}
    />
  )
}

/** The ink panel. A container query sizes the children of `@container`, so the wrapper declares it. */
function Panel({
  children,
  className,
  label = 'ultrafast',
  busy = false,
}: {
  children: ReactNode
  className?: string
  label?: string
  busy?: boolean
}) {
  return (
    <div className={cn('@container', className)}>
      <section
        aria-label={label}
        aria-busy={busy || undefined}
        className={cn(
          'relative rounded-2xl @2xl:px-7 px-[18px] @2xl:pt-[26px] pt-5 @2xl:pb-4 pb-3',
          INK.panel,
          PANEL_SHADOW,
        )}
      >
        {children}
      </section>
    </div>
  )
}

function Meter({
  savings,
  meter,
  tokens,
  stats,
  className,
}: {
  savings: PlanSavings
  meter: PlanSavingsMeter
  tokens: number
  stats: CockpitStats | undefined
  className?: string
}) {
  const note = meterNote(tokens, savings.tasks)
  const fastest =
    meter.fastestRunMs == null ? null : ultrafastTime(meter.fastestRunMs)
  return (
    <Panel className={className}>
      <div className="grid @2xl:grid-cols-[minmax(0,1.75fr)_minmax(0,1fr)] grid-cols-1 @2xl:items-end gap-[22px] @2xl:gap-x-11 @2xl:gap-y-0">
        <div className="min-w-0">
          <div
            className={cn(
              'flex items-baseline gap-2.5 text-[13.5px]',
              INK.ink3,
            )}
          >
            <Wordmark />
            <span>since {monthDay(meter.since)}</span>
          </div>
          <div className="mt-3 flex flex-wrap items-baseline gap-x-3.5 gap-y-1">
            <BigNumber
              tokens={tokens}
              className="@2xl:text-[76px] text-[60px]"
            />
            <span className={cn('text-[15px]', INK.ink2)}>tokens saved</span>
          </div>
          <Scale tokens={tokens} />
          <div
            className={cn(
              'mt-1.5 flex flex-wrap justify-between gap-x-4 gap-y-1 text-[13.5px]',
              INK.ink2,
            )}
          >
            <span>
              <b className={cn('font-bold', INK.ink)}>{note.toGo}</b> to go to{' '}
              {note.next}
            </span>
            {note.passed ? (
              <span className={cn('font-semibold', INK.accent)}>
                {note.passed}
              </span>
            ) : null}
          </div>
        </div>
        <dl className="m-0 flex flex-col">
          <LedgerRow label="Pages neo accelerated">
            {savings.totals.pagesRead}
          </LedgerRow>
          <LedgerRow label="ultrafast runs">{savings.tasks}</LedgerRow>
          {meter.neoStepShare == null ? (
            <LedgerRow label="Steps done by neo" none>
              None yet
            </LedgerRow>
          ) : (
            <LedgerRow label="Steps done by neo" unit="%">
              {meter.neoStepShare}
            </LedgerRow>
          )}
          {fastest === null ? (
            <LedgerRow label="Fastest run" none>
              None yet
            </LedgerRow>
          ) : (
            <LedgerRow label="Fastest run" unit={` ${fastest.split(' ')[1]}`}>
              {fastest.split(' ')[0]}
            </LedgerRow>
          )}
        </dl>
      </div>
      <Foot savings={savings} stats={stats} />
    </Panel>
  )
}

function FirstRun({
  savings,
  tokens,
  stats,
  className,
}: {
  savings: PlanSavings
  tokens: number
  stats: CockpitStats | undefined
  className?: string
}) {
  const [copied, setCopied] = useState(false)
  const next = nextMilestone(tokens)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(TRY_TASK)
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  return (
    <Panel className={className}>
      <div className="grid @2xl:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)] grid-cols-1 @2xl:items-end gap-6 @2xl:gap-x-11">
        <div className="flex flex-col gap-4">
          <Wordmark />
          <h3
            className={cn(
              'm-0 font-bold @2xl:text-[30px] text-[25px] leading-[1.1] tracking-[-0.03em]',
              INK.ink,
            )}
          >
            Let neo take the clicks.
          </h3>
          <p
            className={cn(
              'm-0 max-w-[46ch] text-[14.5px] leading-normal',
              INK.ink2,
            )}
          >
            Give your AI agent a task on a website. neo takes the clicks and
            typing itself, so the pages never fill your agent's context. Your
            first one moves the needle.
          </p>
          <div
            className={cn(
              'flex items-center gap-2.5 rounded-xl border bg-[rgb(238_243_255/0.05)] py-3 pr-3 pl-3.5',
              INK.line,
            )}
          >
            <span
              className={cn('flex-1 text-[13.5px] leading-[1.45]', INK.ink)}
            >
              {TRY_TASK}
            </span>
            <button
              type="button"
              onClick={copy}
              className="inline-flex h-[34px] shrink-0 items-center gap-1.5 rounded-lg bg-[#eef3ff] px-3.5 font-semibold text-[#0c2742] text-[13.5px] hover:bg-white active:translate-y-px"
            >
              <Copy aria-hidden className="size-3.5" />
              {copied ? 'Copied' : 'Copy'}
            </button>
          </div>
          <p className={cn('m-0 text-[13px]', INK.ink3)}>
            Paste it into Claude Code, Codex or Cursor, then come back here.
          </p>
        </div>
        <div className="min-w-0">
          <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
            <BigNumber tokens={tokens} className="text-[48px]" />
            <span className={cn('text-[15px]', INK.ink2)}>tokens saved</span>
          </div>
          <Scale tokens={tokens} />
          <span className={cn('mt-1.5 block text-[13px]', INK.ink3)}>
            {next === 10_000
              ? 'First milestone: 10k. The needle moves as neo saves tokens for you.'
              : `Next milestone: ${meterTokens(next)}. The needle moves as neo saves tokens for you.`}
          </span>
        </div>
      </div>
      <Foot savings={savings} stats={stats} />
    </Panel>
  )
}

function Loading({ className }: { className?: string }) {
  const block = 'block rounded-md bg-[rgb(238_243_255/0.09)]'
  return (
    <Panel className={className} label="Loading your ultrafast savings" busy>
      <div className="grid @2xl:grid-cols-[minmax(0,1.75fr)_minmax(0,1fr)] grid-cols-1 @2xl:items-end gap-[22px] @2xl:gap-x-11">
        <div>
          <Wordmark />
          <span className={cn(block, 'mt-4 h-14 w-[46%]')} />
          <span className="mt-10 block h-px bg-[rgb(238_243_255/0.14)]" />
          <span className={cn(block, 'mt-4 h-3 w-[34%]')} />
        </div>
        <div className="flex flex-col gap-5 pb-2">
          <span className={cn(block, 'h-3.5')} />
          <span className={cn(block, 'h-3.5')} />
          <span className={cn(block, 'h-3.5')} />
          <span className={cn(block, 'h-3.5')} />
        </div>
      </div>
      <div className={cn('mt-5 border-t pt-3 text-[13px]', INK.line, INK.ink3)}>
        Counting what neo saved
      </div>
    </Panel>
  )
}

/** The ruled log scale: ticks 1 to 9 per decade from 1k, milestones marked above, one needle. */
function Scale({ tokens }: { tokens: number }) {
  const top = scaleTop(tokens)
  const at = (value: number) =>
    `${(scalePosition(value, top) * 100).toFixed(3)}%`
  const marks = scaleMarks(tokens, top)
  const lastPassed = marks.filter((mark) => mark.state === 'passed').pop()
  const next = marks.find((mark) => mark.state === 'next')
  return (
    <div
      role="img"
      aria-label={`${meterTokens(tokens)} tokens saved, ${meterTokens(nextMilestone(tokens) - tokens)} to go to the next milestone`}
      className="relative mx-0.5 mt-[26px] h-[46px]"
    >
      <span className="absolute inset-x-0 top-2.5 h-px bg-[rgb(238_243_255/0.26)]" />
      <span
        className="absolute top-2.5 left-0 h-px bg-[rgb(238_243_255/0.82)]"
        style={{ width: at(tokens) }}
      />
      {scaleTicks(top).map((tick) => (
        <span
          key={tick.value}
          className={cn(
            'absolute top-2.5 -ml-[0.5px] w-px',
            tick.decade ? 'h-3' : 'h-1.5',
            tick.value <= tokens
              ? 'bg-[rgb(238_243_255/0.82)]'
              : 'bg-[rgb(238_243_255/0.26)]',
          )}
          style={{ left: at(tick.value) }}
        />
      ))}
      {marks.map((mark) => (
        <svg
          key={mark.value}
          aria-hidden
          data-mark={mark.state}
          viewBox="0 0 9 7"
          className="absolute top-0 -ml-[4.5px] h-[7px] w-[9px] overflow-visible"
          style={{ left: at(mark.value) }}
        >
          <polygon
            points="0.6,0.6 8.4,0.6 4.5,6.4"
            strokeWidth={mark.state === 'next' ? 1.4 : 1.2}
            className={cn(
              mark.state === 'passed' && 'fill-[#eef3ff] stroke-[#eef3ff]',
              mark.state === 'next' && 'fill-none stroke-[#9cc0ff]',
              mark.state === 'ahead' &&
                'fill-none stroke-[rgb(238_243_255/0.26)]',
            )}
          />
        </svg>
      ))}
      {marks.map((mark, index) => (
        <span
          key={mark.value}
          className={cn(
            'absolute top-7 whitespace-nowrap font-mono text-[10.5px] tabular-nums leading-none',
            index === marks.length - 1
              ? '-translate-x-full'
              : '-translate-x-1/2',
            mark.state === 'passed' && INK.ink2,
            mark.state === 'next' && INK.accent,
            mark.state === 'ahead' && INK.ink3,
            // Narrow, only the last milestone passed and the next one keep their numbers.
            mark !== lastPassed && mark !== next && '@max-2xl:hidden',
          )}
          style={{ left: at(mark.value) }}
        >
          {meterTokens(mark.value)}
        </span>
      ))}
      <Needle tokens={tokens} top={top} />
    </div>
  )
}

function Needle({ tokens, top }: { tokens: number; top: number }) {
  const to = scalePosition(tokens, top)
  const [shown, setShown] = useState(() => {
    const last = lastReading()
    return last === null || prefersStill() ? to : scalePosition(last, top)
  })
  // The needle moves once, from the last reading this computer saw; that reading lives in browser storage.
  useEffect(() => {
    rememberReading(tokens)
    if (prefersStill()) {
      setShown(to)
      return
    }
    const frame = requestAnimationFrame(() => setShown(to))
    return () => cancelAnimationFrame(frame)
  }, [tokens, to])
  return (
    <span
      aria-hidden
      data-needle={to.toFixed(4)}
      className="pointer-events-none absolute inset-0 transition-transform duration-[1250ms] ease-[cubic-bezier(0.22,1.25,0.36,1)] motion-reduce:transition-none"
      style={{ transform: `translateX(${shown * 100}%)` }}
    >
      <span className="absolute -top-1.5 -left-px h-[30px] w-0.5 rounded-[1px] bg-[#9cc0ff] after:absolute after:-top-[3px] after:-left-0.5 after:size-1.5 after:rounded-full after:bg-[#9cc0ff] after:content-['']" />
    </span>
  )
}

/** One ledger line: the label, dotted leaders, the figure. */
function LedgerRow({
  label,
  unit,
  none = false,
  children,
}: {
  label: string
  unit?: string
  none?: boolean
  children: ReactNode
}) {
  return (
    <div className="flex items-baseline gap-2 py-[7px]">
      <dt
        className={cn(
          'flex min-w-0 flex-1 items-baseline gap-2 text-[14px]',
          INK.ink2,
        )}
      >
        <span className="whitespace-nowrap">{label}</span>
        <span
          aria-hidden
          className="min-w-3 flex-1 -translate-y-1 border-[rgb(238_243_255/0.32)] border-b border-dotted"
        />
      </dt>
      <dd
        className={cn(
          'm-0 whitespace-nowrap',
          none
            ? cn('font-medium text-[14px]', INK.ink3)
            : cn(
                'font-bold text-[20px] tabular-nums tracking-[-0.02em]',
                INK.ink,
              ),
        )}
      >
        {children}
        {unit ? (
          <small
            className={cn(
              'font-semibold text-[13px] tracking-normal',
              INK.ink2,
            )}
          >
            {unit}
          </small>
        ) : null}
      </dd>
    </div>
  )
}

function Foot({
  savings,
  stats,
}: {
  savings: PlanSavings
  stats: CockpitStats | undefined
}) {
  const time = stats ? roundTimeSaved(stats.allTime.humanTimeSavedMs) : null
  return (
    <div
      className={cn(
        'mt-5 flex flex-wrap items-center gap-x-4 gap-y-1.5 border-t pt-3 text-[13px]',
        INK.line,
        INK.ink3,
      )}
    >
      {stats && time ? (
        <span>
          Also with neo:{' '}
          <b className={cn('font-bold', INK.ink2)}>
            {timeSavedAmount(stats.allTime.humanTimeSavedMs)}
          </b>{' '}
          of your time saved
        </span>
      ) : null}
      <span className="flex-1" />
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            aria-label="How tokens saved is counted"
            className={cn(LINK, 'font-medium', INK.ink2)}
          >
            <Info aria-hidden className="size-3.5" />
            How we count
          </button>
        </TooltipTrigger>
        <TooltipContent>{HOW_WE_COUNT}</TooltipContent>
      </Tooltip>
      <UltrafastDetailsDialog
        savings={savings}
        trigger={
          <button type="button" className={cn(LINK, 'font-semibold', INK.ink)}>
            Details
            <ArrowRight aria-hidden className="size-3.5" />
          </button>
        }
      />
    </div>
  )
}

function prefersStill(): boolean {
  try {
    return window.matchMedia('(prefers-reduced-motion: reduce)').matches
  } catch {
    return true
  }
}
