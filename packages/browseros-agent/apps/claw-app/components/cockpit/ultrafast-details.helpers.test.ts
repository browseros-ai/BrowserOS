/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  axisLabels,
  runsStrip,
  stripCaption,
} from './ultrafast-details.helpers'

const at = (day: number, hour = 12) => new Date(2026, 10, day, hour).getTime()
const NOW = at(17, 18)
const SINCE = at(5, 9)

/** Runs on each listed day of November: { 13: 37 } is 37 runs on Nov 13. */
function times(perDay: Record<number, number>): number[] {
  return Object.entries(perDay).flatMap(([day, runs]) =>
    Array.from({ length: runs }, (_, run) => at(Number(day), 9 + (run % 12))),
  )
}

const LIGHT = { runTimes: times({ 7: 1, 12: 1, 13: 1, 16: 1, 17: 1 }) }
const HEAVY = {
  runTimes: times({
    5: 3,
    6: 11,
    7: 15,
    8: 26,
    9: 40,
    10: 28,
    11: 4,
    13: 19,
    14: 37,
    15: 8,
    16: 31,
    17: 23,
  }),
}

describe('the runs strip', () => {
  it('stacks squares while no day in view passes eight runs', () => {
    const strip = runsStrip('since', LIGHT, SINCE, NOW)
    expect(strip.bars).toBe(false)
    expect(strip.columns).toHaveLength(13)
    expect(strip.columns[0]).toMatchObject({
      label: 'Nov 5',
      purchase: true,
      before: false,
    })
    expect(strip.columns[12]).toMatchObject({
      label: 'Nov 17',
      today: true,
      runs: 1,
    })
    expect(
      strip.columns.map((column) => column.runs).reduce((a, b) => a + b),
    ).toBe(5)
  })

  it('switches the whole strip to bars once any day passes eight', () => {
    const strip = runsStrip('since', HEAVY, SINCE, NOW)
    expect(strip.bars).toBe(true)
    expect(strip.busiest).toBe(40)
    expect(stripCaption(strip)).toBe('Runs per day. The busiest day is marked.')
  })

  it('draws 30 days one column a day, with the days before ultrafast marked', () => {
    const strip = runsStrip('last30Days', HEAVY, SINCE, NOW)
    expect(strip.columns).toHaveLength(30)
    expect(strip.columns.filter((column) => column.before)).toHaveLength(17)
    expect(strip.columns.findIndex((column) => column.purchase)).toBe(17)
  })

  it('names the 7 days by weekday, today last', () => {
    const strip = runsStrip('last7Days', LIGHT, SINCE, NOW)
    expect(strip.columns.map((column) => column.label)).toEqual([
      'Wed',
      'Thu',
      'Fri',
      'Sat',
      'Sun',
      'Mon',
      'Tue',
    ])
    expect(strip.columns.map((column) => column.runs)).toEqual([
      0, 1, 1, 0, 0, 1, 1,
    ])
    expect(stripCaption(strip)).toBe('One square is one ultrafast run.')
  })

  it('folds a since period past a month into weeks', () => {
    const strip = runsStrip(
      'since',
      { runTimes: [] },
      at(5) - 60 * 86_400_000,
      NOW,
    )
    expect(strip.weekly).toBe(true)
    expect(strip.columns.length).toBeLessThan(15)
    expect(strip.columns.filter((column) => column.purchase)).toHaveLength(1)
  })
})

describe('the strip axis', () => {
  it('keeps the purchase day and today, and drops a label that would touch them', () => {
    const strip = runsStrip('since', HEAVY, SINCE, NOW)
    const texts = axisLabels('since', strip, 352).map((label) => label.text)
    expect(texts).toContain('Nov 5')
    expect(texts).toContain('Nov 17')
    // Nov 16 is a Monday right next to today.
    expect(texts).not.toContain('Nov 16')
    expect(axisLabels('since', strip, 352)[0]?.kind).toBe('purchase')
  })

  it('names the stretch before ultrafast once on the 30-day strip', () => {
    const strip = runsStrip('last30Days', HEAVY, SINCE, NOW)
    const labels = axisLabels('last30Days', strip, 500)
    expect(labels.filter((label) => label.kind === 'before')).toEqual([
      expect.objectContaining({ text: 'before ultrafast' }),
    ])
    for (const [index, label] of labels.entries()) {
      const next = labels[index + 1]
      if (next) expect(next.left).toBeGreaterThan(label.left)
    }
  })
})

describe('the since period', () => {
  it('counts a run from the purchase day even when it came before the purchase hour', () => {
    const strip = runsStrip(
      'since',
      { runTimes: [at(5, 7), at(6)] },
      SINCE,
      NOW,
    )
    expect(strip.columns[0]?.purchase).toBe(true)
    expect(strip.columns[0]?.runs).toBe(1)
    expect(
      strip.columns.map((column) => column.runs).reduce((a, b) => a + b),
    ).toBe(2)
  })
})
