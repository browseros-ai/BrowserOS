/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  chart,
  chartBarHeight,
  howWeCount,
  periodStart,
} from './ultrafast-details.helpers'

const at = (day: number, hour = 12) => new Date(2026, 10, day, hour).getTime()
const NOW = at(17, 18)
const SINCE = at(5, 9)

describe('the runs chart', () => {
  it('counts the last 7 days by local day, today last', () => {
    const { title, bars } = chart(
      'last7Days',
      { runTimes: [at(17), at(16), at(13), at(12)] },
      SINCE,
      NOW,
    )
    expect(title).toBe('ultrafast runs, day by day')
    expect(bars.map((bar) => bar.label)).toEqual([
      'Wed',
      'Thu',
      'Fri',
      'Sat',
      'Sun',
      'Mon',
      'Tue',
    ])
    expect(bars.map((bar) => bar.runs)).toEqual([0, 1, 1, 0, 0, 1, 1])
  })

  it('folds 30 days into weeks and outlines the purchase week', () => {
    const { bars, note } = chart(
      'last30Days',
      { runTimes: [at(17), at(16), at(13), at(12), at(7)] },
      SINCE,
      NOW,
    )
    expect(bars).toHaveLength(5)
    expect(bars.map((bar) => bar.runs)).toEqual([0, 0, 0, 1, 4])
    expect(bars[3]?.purchase).toBe(true)
    expect(bars.filter((bar) => bar.purchase)).toHaveLength(1)
    expect(note).toBe('The outlined week is when you got ultrafast (Nov 5).')
  })

  it('draws since day by day from the purchase, labelling the edges and Mondays', () => {
    const { title, bars } = chart('since', { runTimes: [at(7)] }, SINCE, NOW)
    expect(title).toBe('ultrafast runs since Nov 5')
    expect(bars).toHaveLength(13)
    expect(bars[0]?.label).toBe('Nov 5')
    expect(bars[12]?.label).toBe('Nov 17')
    expect(bars[4]?.label).toBe('Nov 9')
    expect(bars[2]?.runs).toBe(1)
  })

  it('switches a long since period to weeks', () => {
    const { bars } = chart(
      'since',
      { runTimes: [] },
      at(5) - 60 * 86_400_000,
      NOW,
    )
    expect(bars.length).toBeLessThan(15)
  })

  it('stubs an empty bar and scales the rest to the busiest', () => {
    expect(chartBarHeight(0, 3)).toBe('12%')
    expect(chartBarHeight(3, 3)).toBe('100%')
    expect(chartBarHeight(1, 2)).toBe('65%')
  })
})

describe('periods', () => {
  it('start at the purchase or 30 and 7 days back', () => {
    expect(periodStart('since', SINCE, NOW)).toBe(SINCE)
    expect(NOW - periodStart('last7Days', SINCE, NOW)).toBe(7 * 86_400_000)
    expect(NOW - periodStart('last30Days', SINCE, NOW)).toBe(30 * 86_400_000)
  })

  it('word how we count for each tab', () => {
    expect(howWeCount('since', SINCE)).toMatch(/^Since Nov 5, tokens saved/)
    expect(howWeCount('last30Days', SINCE)).toMatch(/^In the last 30 days,/)
    expect(howWeCount('last7Days', SINCE)).toMatch(/^In the last 7 days,/)
  })
})
