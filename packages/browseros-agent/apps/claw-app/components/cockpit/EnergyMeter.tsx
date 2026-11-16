/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Home for an ultrafast customer: one ring of total tokens saved since this
 * computer got ultrafast, the next milestone, four tiles and the week's runs.
 * Free installs keep SavingsCard.
 */

import type {
  CockpitStats,
  PlanSavings,
  PlanSavingsMeter,
} from '@browseros/claw-api'
import type { UseQueryResult } from '@tanstack/react-query'
import { ArrowRight, Copy, Info, Trophy, Zap } from 'lucide-react'
import { type ReactNode, useEffect, useState } from 'react'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/motion/tooltip'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { ultrafastTime } from '@/screens/audit/audit.helpers'
import { roundTimeSaved } from '@/screens/cockpit/cockpit.helpers'
import {
  barHeight,
  type MilestoneCard,
  meterTokens,
  milestoneCard,
  milestoneCopy,
  nextMilestone,
  rememberSpark,
  sparkSeen,
  weekRuns,
} from './energy-meter.helpers'
import { SavingsDialog } from './SavingsCard'
import { timeSavedAmount } from './savings.helpers'

const RADIUS = 104
const CIRCUMFERENCE = 2 * Math.PI * RADIUS
const INK_2 = 'text-[rgb(214_225_255/0.78)]'
const INK_3 = 'text-[rgb(190_205_245/0.58)]'

export const TRY_TASK =
  'On google.com/travel/flights, search one-way flights from Chennai to Singapore on 20 December and sort by price.'

const HOW_WE_COUNT =
  'Tokens saved adds up everything neo kept off your agent since you got ultrafast: short page descriptions instead of screenshots, and pages neo read itself. The two are measured differently, so the total is an estimate.'

export function EnergyMeter({
  savings,
  stats,
  className,
}: {
  savings: UseQueryResult<PlanSavings>
  stats: CockpitStats | undefined
  className?: string
}) {
  if (savings.isPending) {
    return (
      <Skeleton
        aria-label="Loading your ultrafast savings"
        className={cn('h-[300px] rounded-[20px]', className)}
      />
    )
  }
  const meter = savings.data?.meter
  if (savings.isError || !meter) {
    return (
      <section
        aria-label="ultrafast"
        className={cn('em flex flex-col gap-3 px-8 py-7', className)}
      >
        <span aria-hidden className="em-grain" />
        <Mark />
        <p role="alert" className={cn('text-[14.5px]', INK_2)}>
          Your ultrafast numbers didn't load.
        </p>
        <button
          type="button"
          onClick={() => savings.refetch()}
          className="em-chip h-[34px] self-start rounded-[10px] px-3.5 font-semibold text-[13.5px] text-white"
        >
          Try again
        </button>
      </section>
    )
  }
  const plan = savings.data
  if (plan.tasks === 0) {
    return <FirstRun meter={meter} stats={stats} className={className} />
  }
  return (
    <Meter
      meter={meter}
      pages={plan.totals.pagesRead}
      runs={plan.tasks}
      stats={stats}
      className={className}
    />
  )
}

function Meter({
  meter,
  pages,
  runs,
  stats,
  className,
}: {
  meter: PlanSavingsMeter
  pages: number
  runs: number
  stats: CockpitStats | undefined
  className?: string
}) {
  const card = milestoneCard(meter.tokensSaved, runs)
  const week = weekRuns(meter.runTimes, Date.now())

  return (
    // A container query sizes the children of `@container`, so the panel's own columns need it on a wrapper.
    <div className={cn('@container', className)}>
      <section
        aria-label="ultrafast"
        className={cn(
          'em grid grid-cols-1 justify-items-center gap-[22px] px-5 pt-6 pb-5',
          '@2xl:grid-cols-[auto_1fr] @2xl:items-center @2xl:justify-items-stretch @2xl:gap-[34px] @2xl:px-8 @2xl:pt-[30px] @2xl:pb-6',
        )}
      >
        <span aria-hidden className="em-grain" />
        <Ring tokens={meter.tokensSaved} />
        <div className="flex w-full min-w-0 flex-col gap-[18px]">
          <div className="flex flex-wrap items-center justify-between gap-2.5">
            <span
              className={cn(
                'em-chip inline-flex h-[26px] items-center rounded-full px-2.5 font-semibold text-[12.5px]',
                INK_2,
              )}
            >
              since you got ultrafast
            </span>
          </div>
          {/* Keyed on the milestone so one crossed while Home is open mounts fresh and plays its burst. */}
          <Milestone
            key={card.kind === 'quiet' ? 'quiet' : card.key}
            card={card}
          />
          <div className="grid @2xl:grid-cols-4 grid-cols-2 gap-2.5">
            <Tile
              value={<CountUp to={pages} />}
              label="pages neo accelerated"
            />
            <Tile value={<CountUp to={runs} />} label="ultrafast runs" />
            <Tile
              value={
                meter.neoStepShare == null ? (
                  'None yet'
                ) : (
                  <>
                    <CountUp to={meter.neoStepShare} />%
                  </>
                )
              }
              label="of the steps, done by neo"
            />
            <Tile
              value={
                meter.fastestRunMs == null
                  ? 'None yet'
                  : ultrafastTime(meter.fastestRunMs)
              }
              label="your fastest ultrafast run"
            />
          </div>
          <div
            role="img"
            aria-label={`ultrafast runs this week: ${week.map((day) => `${day.label} ${day.runs}`).join(', ')}`}
            className="flex flex-col gap-1.5"
          >
            <div className="flex h-[34px] items-end gap-1.5">
              {week.map((day, index) => (
                <span
                  key={day.label}
                  data-hit={day.runs > 0 ? '' : undefined}
                  className="em-bar flex-1"
                  style={
                    {
                      height: barHeight(day.runs),
                      '--i': index,
                    } as React.CSSProperties
                  }
                />
              ))}
            </div>
            <div className={cn('flex gap-1.5 text-[11px]', INK_3)}>
              {week.map((day) => (
                <span key={day.label} className="flex-1 text-center">
                  {day.label}
                </span>
              ))}
            </div>
          </div>
        </div>
        <Fold stats={stats} className="col-span-full w-full" />
      </section>
    </div>
  )
}

function FirstRun({
  meter,
  stats,
  className,
}: {
  meter: PlanSavingsMeter
  stats: CockpitStats | undefined
  className?: string
}) {
  const [copied, setCopied] = useState(false)
  const next = nextMilestone(meter.tokensSaved)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(TRY_TASK)
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  return (
    <div className={cn('@container', className)}>
      <section
        aria-label="ultrafast"
        className="em grid @2xl:grid-cols-[1.1fr_1fr] grid-cols-1"
      >
        <span aria-hidden className="em-grain" />
        <div className="flex flex-col gap-4 @2xl:px-[30px] px-5 @2xl:py-7 py-[22px]">
          <Mark />
          <h3 className="m-0 font-extrabold @2xl:text-[30px] text-[25px] text-white leading-[1.1] tracking-[-0.03em]">
            Let neo take the clicks.
          </h3>
          <p
            className={cn(
              'm-0 max-w-[46ch] text-[14.5px] leading-normal',
              INK_2,
            )}
          >
            Give your AI agent a task on a website. neo takes the clicks and
            typing itself, so the pages never fill your agent's context. Your
            first one lights this up.
          </p>
          <div className="em-prompt flex items-center gap-2.5 rounded-xl py-3 pr-3 pl-3.5">
            <span className="flex-1 text-[13.5px] text-white leading-[1.45]">
              {TRY_TASK}
            </span>
            <button
              type="button"
              onClick={copy}
              className="inline-flex h-[34px] shrink-0 items-center gap-1.5 rounded-[10px] bg-white px-3.5 font-semibold text-[#01123f] text-[13.5px] hover:bg-[#eaf1ff]"
            >
              <Copy aria-hidden className="size-3.5" />
              {copied ? 'Copied' : 'Copy'}
            </button>
          </div>
          <p className={cn('m-0 text-[13px]', INK_3)}>
            Paste it into Claude Code, Codex or Cursor, then come back here.
          </p>
        </div>
        <div className="em-aside flex flex-col items-center justify-center gap-3 border-t @2xl:border-t-0 @2xl:border-l @2xl:px-[30px] px-5 @2xl:py-7 py-[22px]">
          <Ring tokens={meter.tokensSaved} size="small" />
          <span className={cn('text-center text-[13px]', INK_3)}>
            {next === 10_000
              ? 'First milestone: 10k. The ring fills as neo saves tokens for you.'
              : `Next milestone: ${meterTokens(next)}. The ring fills as neo saves tokens for you.`}
          </span>
        </div>
        <Fold
          stats={stats}
          className="col-span-full @2xl:px-[30px] px-5 pb-[22px]"
        />
      </section>
    </div>
  )
}

// ---------------------------------------------------------------------------

function Mark() {
  return (
    <span className="em-mark inline-flex items-center gap-2 font-extrabold text-[15px] text-white italic tracking-[-0.01em]">
      <Zap aria-hidden className="size-[18px]" />
      ultrafast
    </span>
  )
}

function Ring({
  tokens,
  size = 'large',
}: {
  tokens: number
  size?: 'large' | 'small'
}) {
  const fraction = Math.min(1, tokens / nextMilestone(tokens))
  return (
    <div
      className={cn(
        'relative',
        size === 'small' ? 'size-[200px]' : '@2xl:size-[240px] size-[220px]',
      )}
    >
      <svg
        aria-hidden
        viewBox="0 0 240 240"
        className="em-ring size-full -rotate-90 overflow-visible"
      >
        <defs>
          <linearGradient id="em-ring-gradient" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor="#ffffff" />
            <stop offset=".45" stopColor="#9cc0ff" />
            <stop offset="1" stopColor="#2f6fe0" />
          </linearGradient>
        </defs>
        <circle className="em-track" cx="120" cy="120" r={RADIUS} />
        <circle className="em-ticks" cx="120" cy="120" r={RADIUS} />
        {fraction > 0 ? (
          <circle
            className="em-fill"
            cx="120"
            cy="120"
            r={RADIUS}
            strokeDasharray={CIRCUMFERENCE}
            strokeDashoffset={CIRCUMFERENCE * (1 - fraction)}
            style={
              { '--em-circumference': CIRCUMFERENCE } as React.CSSProperties
            }
          />
        ) : null}
      </svg>
      <div className="absolute inset-0 flex flex-col items-center justify-center gap-1 text-center">
        <Mark />
        <span
          className={cn(
            'em-big font-extrabold leading-[0.92] tracking-[-0.045em]',
            size === 'small' ? 'text-[48px]' : 'text-[56px]',
          )}
        >
          <CountUp to={tokens} format={meterTokens} />
        </span>
        <span
          className={cn('max-w-[140px] text-[12.5px] leading-[1.35]', INK_2)}
        >
          tokens saved
        </span>
      </div>
    </div>
  )
}

function Milestone({ card }: { card: MilestoneCard }) {
  const key = card.kind === 'quiet' ? null : card.key
  // Read once per mount so the burst plays for this view even after it is remembered.
  const [burst] = useState(() => key !== null && !sparkSeen(key))
  // Browser storage sits outside React: remember the milestone once it has been shown.
  useEffect(() => {
    if (key !== null) rememberSpark(key)
  }, [key])
  const { title, line } = milestoneCopy(card)
  const quiet = card.kind === 'quiet'

  return (
    <div
      data-quiet={quiet ? '' : undefined}
      className="em-milestone relative flex items-center gap-3 rounded-[14px] px-3.5 py-3"
    >
      {quiet ? null : (
        <span
          aria-hidden
          data-burst={burst ? '' : undefined}
          className="em-sparks pointer-events-none absolute top-1/2 left-[30px] size-0"
        >
          {Array.from({ length: 14 }, (_, index) => {
            const angle = (index / 14) * Math.PI * 2
            return (
              <i
                // biome-ignore lint/suspicious/noArrayIndexKey: a fixed ring of sparks
                key={index}
                style={
                  {
                    '--dx': `${Math.cos(angle) * 46}px`,
                    '--dy': `${Math.sin(angle) * 30}px`,
                    animationDelay: `calc(1.9s + ${(index % 3) * 40}ms)`,
                  } as React.CSSProperties
                }
              />
            )
          })}
        </span>
      )}
      <span className="em-medal grid size-[34px] shrink-0 place-items-center rounded-full">
        {card.kind === 'passed' ? (
          <Trophy aria-hidden className="size-[17px]" />
        ) : (
          <Zap aria-hidden className="size-[17px]" />
        )}
      </span>
      <span>
        <span className="block font-bold text-[14px] text-white">{title}</span>
        <span className={cn('block text-[12.5px]', INK_2)}>{line}</span>
      </span>
    </div>
  )
}

function Tile({ value, label }: { value: ReactNode; label: string }) {
  return (
    <div className="em-tile flex flex-col gap-0.5 rounded-xl px-3.5 py-3">
      <span className="font-extrabold text-[26px] text-white tabular-nums tracking-[-0.03em]">
        {value}
      </span>
      <span className={cn('text-[12.5px]', INK_3)}>{label}</span>
    </div>
  )
}

function Fold({
  stats,
  className,
}: {
  stats: CockpitStats | undefined
  className?: string
}) {
  const time = stats ? roundTimeSaved(stats.allTime.humanTimeSavedMs) : null
  return (
    <div
      className={cn(
        'em-fold flex flex-wrap items-center gap-x-4 gap-y-1.5 pt-3.5 text-[13px]',
        INK_3,
        className,
      )}
    >
      {stats && time ? (
        <>
          <span>Also with neo:</span>
          <span>
            <b className={cn('font-bold', INK_2)}>
              {timeSavedAmount(stats.allTime.humanTimeSavedMs)}
            </b>{' '}
            of your time saved
          </span>
        </>
      ) : null}
      <span className="flex-1" />
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            aria-label="How tokens saved is counted"
            className={cn(
              'inline-flex items-center gap-1.5 font-semibold text-[13px]',
              INK_2,
            )}
          >
            <Info aria-hidden className="size-3.5" />
            How we count
          </button>
        </TooltipTrigger>
        <TooltipContent>{HOW_WE_COUNT}</TooltipContent>
      </Tooltip>
      {stats ? (
        <SavingsDialog
          stats={stats}
          trigger={
            <button
              type="button"
              className={cn(
                'inline-flex items-center gap-1.5 font-semibold text-[13px]',
                INK_2,
              )}
            >
              Details
              <ArrowRight aria-hidden className="size-3.5" />
            </button>
          }
        />
      ) : null}
    </div>
  )
}

/** Counts up to the number once on mount; readers who prefer less motion see it at once. */
function CountUp({
  to,
  format = (value: number) => String(Math.round(value)),
}: {
  to: number
  format?: (value: number) => string
}) {
  const [shown, setShown] = useState(() => (prefersStill() ? to : 0))
  // requestAnimationFrame drives the count, an external timer React can't express.
  useEffect(() => {
    if (prefersStill()) {
      setShown(to)
      return
    }
    const start = performance.now()
    const duration = to >= 1_000 ? 1_600 : 1_100
    let frame = requestAnimationFrame(function tick(now) {
      const progress = Math.min(1, (now - start) / duration)
      setShown(to * (1 - (1 - progress) ** 4))
      if (progress < 1) frame = requestAnimationFrame(tick)
    })
    return () => cancelAnimationFrame(frame)
  }, [to])
  return <>{format(shown)}</>
}

function prefersStill(): boolean {
  try {
    return window.matchMedia('(prefers-reduced-motion: reduce)').matches
  } catch {
    return true
  }
}
