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

export const DETAILS_HOW_WE_COUNT =
  'Tokens saved adds up page descriptions neo gave your agent instead of screenshots, and pages neo read itself. They are measured differently, so the total is an estimate.'

const DAY_MS = 24 * 60 * 60 * 1000
/** Past this many days, the since strip folds into weeks so its columns stay readable. */
const DAILY_LIMIT = 31
/** A day stacks one square per run up to this many; past it the whole strip draws bars. */
export const SQUARE_LIMIT = 8
/** The tallest bar, in px. */
export const BAR_MAX = 66
/** "Nov 5". */
export function monthDay(epochMs: number): string {
  return new Date(epochMs).toLocaleDateString('en-US', {
    month: 'short',
    day: 'numeric',
  })
}

export interface RunsColumn {
  runs: number
  /** What the hover and the axis say: "Nov 5", "Wed" or "Week of Nov 3". */
  label: string
  /** Before the customer got ultrafast, when no ultrafast run could happen. */
  before: boolean
  /** The day (or week) the customer got ultrafast. */
  purchase: boolean
  today: boolean
  monday: boolean
}

export interface RunsStrip {
  columns: RunsColumn[]
  busiest: number
  /** Past SQUARE_LIMIT runs on any day in view, every column is a bar. */
  bars: boolean
  weekly: boolean
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

/** The period's runs, one column per local day; a since period past a month goes by week. */
export function runsStrip(
  key: PeriodKey,
  period: Pick<PlanSavingsPeriod, 'runTimes'>,
  since: number,
  now: number,
): RunsStrip {
  const times = period.runTimes
  const today = startOfDay(now)
  const bought = startOfDay(since)
  const span =
    key === 'last7Days'
      ? 7
      : key === 'last30Days'
        ? 30
        : Math.max(
            1,
            Math.round((today.getTime() - bought.getTime()) / DAY_MS) + 1,
          )
  const weekly = key === 'since' && span > DAILY_LIMIT
  const weeks = Math.ceil(span / 7)
  const columns: RunsColumn[] = weekly
    ? Array.from({ length: weeks }, (_, index) => {
        const back = weeks - 1 - index
        const start = addDays(today, -(back * 7 + 6))
        const end = addDays(today, -(back * 7) + 1)
        return {
          runs: runsBetween(times, start, end),
          label: `Week of ${monthDay(start.getTime())}`,
          before: false,
          purchase:
            bought.getTime() >= start.getTime() &&
            bought.getTime() < end.getTime(),
          today: back === 0,
          monday: false,
        }
      })
    : Array.from({ length: span }, (_, index) => {
        const start = addDays(today, index - (span - 1))
        return {
          runs: runsBetween(times, start, addDays(start, 1)),
          label:
            key === 'last7Days'
              ? start.toLocaleDateString('en-US', { weekday: 'short' })
              : monthDay(start.getTime()),
          before: start.getTime() < bought.getTime(),
          purchase: start.getTime() === bought.getTime(),
          today: index === span - 1,
          monday: start.getDay() === 1,
        }
      })
  const busiest = Math.max(0, ...columns.map((column) => column.runs))
  return { columns, busiest, bars: busiest > SQUARE_LIMIT, weekly }
}

/** A bar's height in px against the busiest column. */
export function barPx(runs: number, busiest: number): number {
  return Math.max(3, Math.round((runs / Math.max(1, busiest)) * BAR_MAX))
}

/** The strip's height in px: room for the tallest square stack, or for bars and their counts. */
export function stripPx(strip: RunsStrip): number {
  if (strip.bars) return BAR_MAX + 18
  return Math.max(26, 10 + 9 * strip.busiest)
}

export interface AxisLabel {
  /** Left edge, as a fraction of the strip's width. */
  left: number
  text: string
  kind: 'purchase' | 'before' | 'plain'
}

/**
 * Axis labels at this width, by priority: the purchase day, today, the days before ultrafast,
 * then Mondays (every day on the 7-day tab). A label that would touch one already placed is dropped.
 */
export function axisLabels(
  key: PeriodKey,
  strip: RunsStrip,
  widthPx: number,
): AxisLabel[] {
  const columnPx = widthPx / strip.columns.length
  const wanted: {
    rank: number
    centre: number
    text: string
    kind: AxisLabel['kind']
    width?: number
  }[] = []
  strip.columns.forEach((column, index) => {
    const centre = (index + 0.5) * columnPx
    const text = strip.weekly
      ? column.label.replace('Week of ', '')
      : column.label
    if (column.before) return
    if (column.purchase && key !== 'last7Days') {
      wanted.push({ rank: 0, centre, text, kind: 'purchase' })
    } else if (column.today) {
      wanted.push({ rank: 1, centre, text, kind: 'plain' })
    } else if (key === 'last7Days' || strip.weekly || column.monday) {
      wanted.push({ rank: 3, centre, text, kind: 'plain' })
    }
  })
  const before = strip.columns.filter((column) => column.before).length
  if (before > 0) {
    wanted.push({
      rank: 2,
      centre: (before / 2) * columnPx,
      text: 'before ultrafast',
      kind: 'before',
      width: 92,
    })
  }
  const placed: {
    left: number
    width: number
    text: string
    kind: AxisLabel['kind']
  }[] = []
  for (const label of wanted.sort((a, b) => a.rank - b.rank)) {
    const width = label.width ?? label.text.length * 6.4
    const left = Math.min(
      Math.max(label.centre - width / 2, 0),
      widthPx - width,
    )
    const touches = placed.some(
      (other) =>
        left < other.left + other.width + 8 && left + width + 8 > other.left,
    )
    if (!touches)
      placed.push({ left, width, text: label.text, kind: label.kind })
  }
  return placed
    .sort((a, b) => a.left - b.left)
    .map(({ left, text, kind }) => ({ left: left / widthPx, text, kind }))
}

/** The strip's one-line caption. */
export function stripCaption(strip: RunsStrip): string {
  const unit = strip.weekly ? 'week' : 'day'
  return strip.bars
    ? `Runs per ${unit}. The busiest ${unit} is marked.`
    : 'One square is one ultrafast run.'
}
