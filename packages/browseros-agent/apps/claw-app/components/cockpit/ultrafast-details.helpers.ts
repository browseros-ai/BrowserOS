/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { PlanSavingsPeriod } from '@browseros/claw-api'

export type PeriodKey = 'since' | 'last30Days' | 'last7Days'

export const PERIODS: readonly { key: PeriodKey; label: string }[] = [
  { key: 'since', label: 'Since you got ultrafast' },
  { key: 'last30Days', label: '30 days' },
  { key: 'last7Days', label: '7 days' },
]

const DAY_MS = 24 * 60 * 60 * 1000
/** Past this many days, the since chart folds into weeks so its bars stay readable. */
const DAILY_LIMIT = 31

/** "Nov 5". */
export function monthDay(epochMs: number): string {
  return new Date(epochMs).toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
  })
}

/** Where the period starts: the purchase, or 30 and 7 days back. */
export function periodStart(key: PeriodKey, since: number, now: number) {
  if (key === 'since') return since
  return now - (key === 'last30Days' ? 30 : 7) * DAY_MS
}

export function howWeCount(key: PeriodKey, since: number): string {
  const lead =
    key === 'since'
      ? `Since ${monthDay(since)}`
      : key === 'last30Days'
        ? 'In the last 30 days'
        : 'In the last 7 days'
  return `${lead}, tokens saved adds up page descriptions neo gave your agent instead of screenshots and pages neo read itself; they're measured differently, so it's an estimate.`
}

export interface ChartBar {
  runs: number
  label: string
  /** The bar for the week the customer got ultrafast. */
  purchase: boolean
}

export interface Chart {
  title: string
  bars: ChartBar[]
  note: string | null
}

function startOfDay(epochMs: number): Date {
  const day = new Date(epochMs)
  day.setHours(0, 0, 0, 0)
  return day
}

function addDays(day: Date, days: number): Date {
  const next = new Date(day)
  next.setDate(day.getDate() + days)
  return next
}

function runsBetween(times: readonly number[], from: Date, to: Date): number {
  return times.filter((at) => at >= from.getTime() && at < to.getTime()).length
}

/** Seven-day buckets ending today, oldest first. */
function weekBars(
  times: readonly number[],
  weeks: number,
  now: number,
  since: number,
): ChartBar[] {
  const today = startOfDay(now)
  return Array.from({ length: weeks }, (_, index) => {
    const back = weeks - 1 - index
    const start = addDays(today, -(back * 7 + 6))
    const end = addDays(today, -(back * 7) + 1)
    return {
      runs: runsBetween(times, start, end),
      label: monthDay(start.getTime()),
      purchase: since >= start.getTime() && since < end.getTime(),
    }
  })
}

/** The period's runs as bars: by day for 7 days and since, by week for 30 days. */
export function chart(
  key: PeriodKey,
  period: Pick<PlanSavingsPeriod, 'runTimes'>,
  since: number,
  now: number,
): Chart {
  const times = period.runTimes
  const today = startOfDay(now)
  if (key === 'last30Days') {
    const bars = weekBars(times, 5, now, since)
    return {
      title: 'ultrafast runs, week by week',
      bars,
      note: bars.some((bar) => bar.purchase)
        ? `The outlined week is when you got ultrafast (${monthDay(since)}).`
        : null,
    }
  }
  if (key === 'last7Days') {
    return {
      title: 'ultrafast runs, day by day',
      bars: Array.from({ length: 7 }, (_, index) => {
        const start = addDays(today, index - 6)
        return {
          runs: runsBetween(times, start, addDays(start, 1)),
          label: start.toLocaleDateString('en-US', { weekday: 'short' }),
          purchase: false,
        }
      }),
      note: null,
    }
  }
  const first = startOfDay(since)
  const days = Math.round((today.getTime() - first.getTime()) / DAY_MS) + 1
  const title = `ultrafast runs since ${monthDay(since)}`
  if (days > DAILY_LIMIT) {
    return {
      title,
      bars: weekBars(times, Math.ceil(days / 7), now, since),
      note: null,
    }
  }
  return {
    title,
    bars: Array.from({ length: Math.max(1, days) }, (_, index) => {
      const start = addDays(first, index)
      const edge = index === 0 || index === days - 1 || start.getDay() === 1
      return {
        runs: runsBetween(times, start, addDays(start, 1)),
        label: edge ? monthDay(start.getTime()) : '',
        purchase: false,
      }
    }),
    note: null,
  }
}

/** A bar's height against the busiest one; an empty day is a stub. */
export function chartBarHeight(runs: number, busiest: number): string {
  if (runs === 0) return '12%'
  return `${Math.round(30 + (runs / Math.max(1, busiest)) * 70)}%`
}
