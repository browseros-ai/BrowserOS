/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  barHeight,
  meterTokens,
  milestoneCard,
  milestoneCopy,
  nextMilestone,
  weekRuns,
} from './energy-meter.helpers'

const copy = (tokens: number, runs = 5) =>
  milestoneCopy(milestoneCard(tokens, runs))

describe('the milestone card', () => {
  it('celebrates the first ultrafast run on its own', () => {
    expect(copy(6_118, 1)).toEqual({
      title: 'Your first ultrafast run is in',
      line: '6.1k tokens saved since you got ultrafast. Next stop: 10k.',
    })
  })

  it('names each milestone passed, until halfway to the next', () => {
    expect(copy(10_400).title).toBe('You passed 10k tokens saved')
    expect(copy(101_500)).toEqual({
      title: 'You passed 100k tokens saved',
      line: "That's 100k tokens your agent never had to spend since you got ultrafast.",
    })
    expect(copy(253_678).title).toBe('You passed 250k tokens saved')
    expect(copy(503_000)).toEqual({
      title: 'Half a million tokens saved',
      line: '500k tokens your agent never had to spend since you got ultrafast.',
    })
    expect(copy(1_004_000)).toEqual({
      title: 'A million tokens saved',
      line: '1M tokens your agent never had to spend since you got ultrafast.',
    })
  })

  it('counts down quietly once the total is past halfway', () => {
    expect(copy(61_000)).toEqual({
      title: '39k to go to 100k',
      line: 'Every task neo helps your agent with moves this along.',
    })
    expect(milestoneCard(61_000, 5).kind).toBe('quiet')
    // Below the first milestone after more than one run, there is nothing new to celebrate.
    expect(copy(4_000, 3).title).toBe('6.0k to go to 10k')
  })

  it('keeps finding a next milestone past the last named one', () => {
    expect(nextMilestone(2_600_000)).toBe(5_000_000)
    expect(copy(2_700_000).title).toBe('You passed 2.5M tokens saved')
  })
})

describe('meter numbers', () => {
  it('shortens tokens the way the meter shows them', () => {
    expect(meterTokens(640)).toBe('640')
    expect(meterTokens(6_118)).toBe('6.1k')
    expect(meterTokens(253_678)).toBe('254k')
    expect(meterTokens(1_004_000)).toBe('1M')
    expect(meterTokens(3_240_000)).toBe('3.2M')
  })

  it('buckets the week by local day, today last', () => {
    const now = new Date(2026, 10, 16, 18, 0).getTime()
    const today = new Date(2026, 10, 16, 9, 0).getTime()
    const twoDaysAgo = new Date(2026, 10, 14, 23, 30).getTime()
    const week = weekRuns([today, today, twoDaysAgo], now)
    expect(week).toHaveLength(7)
    expect(week[6]).toEqual({ label: 'Mon', runs: 2 })
    expect(week[4]).toEqual({ label: 'Sat', runs: 1 })
    expect(week.reduce((sum, day) => sum + day.runs, 0)).toBe(3)
  })

  it('draws an empty day as a stub and caps a busy one', () => {
    expect(barHeight(0)).toBe('14%')
    expect(barHeight(1)).toBe('65%')
    expect(barHeight(5)).toBe('100%')
  })
})
