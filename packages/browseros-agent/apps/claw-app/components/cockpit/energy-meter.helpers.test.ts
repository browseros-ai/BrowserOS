/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  meterNote,
  meterParts,
  meterTokens,
  nextMilestone,
  scaleMarks,
  scalePosition,
  scaleTicks,
  scaleTop,
} from './energy-meter.helpers'

describe('the line under the scale', () => {
  it('says the first ultrafast run is in', () => {
    expect(meterNote(6_118, 1)).toEqual({
      toGo: '3.9k',
      next: '10k',
      passed: 'Your first ultrafast run is in',
    })
  })

  it('says a milestone was just passed until halfway to the next, then passed', () => {
    expect(meterNote(253_678, 5).passed).toBe('You just passed 250k')
    expect(meterNote(253_678, 5).toGo).toBe('246k')
    expect(meterNote(326_060, 5).passed).toBe('You just passed 250k')
    expect(meterNote(400_000, 7)).toEqual({
      toGo: '100k',
      next: '500k',
      passed: 'Passed 250k',
    })
    expect(meterNote(101_500, 3).passed).toBe('You just passed 100k')
    expect(meterNote(1_004_000, 9).passed).toBe('You just passed 1M')
  })

  it('says nothing about milestones below the first after more than one run', () => {
    expect(meterNote(4_000, 3)).toEqual({
      toGo: '6.0k',
      next: '10k',
      passed: null,
    })
  })

  it('keeps finding a next milestone past the last named one', () => {
    expect(nextMilestone(2_600_000)).toBe(5_000_000)
    expect(meterNote(2_700_000, 40).passed).toBe('You just passed 2.5M')
  })
})

describe('the log scale', () => {
  it('runs from 1k to 2.5M, or to the next milestone past it', () => {
    expect(scaleTop(326_060)).toBe(2_500_000)
    expect(scaleTop(5_300_000)).toBe(7_500_000)
    expect(scalePosition(1_000, 2_500_000)).toBe(0)
    expect(scalePosition(500, 2_500_000)).toBe(0)
    expect(scalePosition(2_500_000, 2_500_000)).toBe(1)
    // Each decade is as wide as the next, the way a slide rule is ruled.
    const decade = scalePosition(10_000, 2_500_000)
    expect(scalePosition(100_000, 2_500_000)).toBeCloseTo(decade * 2, 6)
  })

  it('rules 1 to 9 in every decade and marks each power of ten', () => {
    const ticks = scaleTicks(2_500_000)
    expect(ticks[0]).toEqual({ value: 1_000, decade: true })
    expect(
      ticks.filter((tick) => tick.decade).map((tick) => tick.value),
    ).toEqual([1_000, 10_000, 100_000, 1_000_000, 2_500_000])
    expect(ticks).toHaveLength(30)
  })

  it('marks milestones passed, the next one, and the rest ahead', () => {
    const marks = scaleMarks(326_060, 2_500_000)
    expect(marks.map((mark) => mark.state)).toEqual([
      'passed',
      'passed',
      'passed',
      'next',
      'ahead',
      'ahead',
    ])
    expect(scaleMarks(5_300_000, 7_500_000).map((mark) => mark.value)).toEqual([
      10_000, 100_000, 250_000, 500_000, 1_000_000, 2_500_000, 5_000_000,
      7_500_000,
    ])
  })
})

describe('meter numbers', () => {
  it('shortens tokens the way the meter shows them', () => {
    expect(meterTokens(640)).toBe('640')
    expect(meterTokens(6_118)).toBe('6.1k')
    expect(meterTokens(253_678)).toBe('254k')
    expect(meterTokens(1_004_000)).toBe('1M')
    expect(meterTokens(3_240_000)).toBe('3.2M')
    expect(meterTokens(4_180)).toBe('4.2k')
    expect(meterTokens(9_960)).toBe('10k')
    expect(meterTokens(972_900)).toBe('973k')
    expect(meterTokens(999_600)).toBe('1M')
    expect(meterTokens(1_240_000)).toBe('1.2M')
  })

  it('splits the figure from its unit so the unit can be set smaller', () => {
    expect(meterParts(253_678)).toEqual({ figure: '254', unit: 'k' })
    expect(meterParts(1_850_000)).toEqual({ figure: '1.9', unit: 'M' })
    expect(meterParts(640)).toEqual({ figure: '640', unit: '' })
  })
})
