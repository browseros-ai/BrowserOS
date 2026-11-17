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
  passedMilestones,
  scalePosition,
  scaleSpan,
  scaleTicks,
  trailCount,
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

describe('the milestones', () => {
  it('step 10k, 25k, 50k in every power of ten and never run out', () => {
    expect(passedMilestones(12_000_000)).toEqual([
      10_000, 25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000, 2_500_000,
      5_000_000, 10_000_000,
    ])
    expect(nextMilestone(0)).toBe(10_000)
    expect(nextMilestone(10_000)).toBe(25_000)
    expect(nextMilestone(30_000_000)).toBe(50_000_000)
    expect(nextMilestone(400_000_000)).toBe(500_000_000)
  })
})

describe('the close-up scale', () => {
  // total, stretch, needle, milestones in the trail, the line under the scale.
  const stages = [
    [
      6_000,
      [0, 10_000],
      0.6,
      0,
      '4.0k',
      '10k',
      'Your first ultrafast run is in',
    ],
    [38_000, [25_000, 50_000], 0.52, 1, '12k', '50k', 'Passed 25k'],
    [
      120_000,
      [100_000, 250_000],
      0.1333,
      3,
      '130k',
      '250k',
      'You just passed 100k',
    ],
    [
      366_000,
      [250_000, 500_000],
      0.464,
      4,
      '134k',
      '500k',
      'You just passed 250k',
    ],
    [
      740_000,
      [500_000, 1_000_000],
      0.48,
      5,
      '260k',
      '1M',
      'You just passed 500k',
    ],
    [
      1_600_000,
      [1_000_000, 2_500_000],
      0.4,
      6,
      '900k',
      '2.5M',
      'You just passed 1M',
    ],
    [4_200_000, [2_500_000, 5_000_000], 0.68, 7, '800k', '5M', 'Passed 2.5M'],
    [
      12_000_000,
      [10_000_000, 25_000_000],
      0.1333,
      9,
      '13M',
      '25M',
      'You just passed 10M',
    ],
  ] as const

  for (const [
    tokens,
    [from, to],
    needle,
    trail,
    toGo,
    next,
    passed,
  ] of stages) {
    it(`spans the current stretch at ${meterTokens(tokens)}`, () => {
      const span = scaleSpan(tokens)
      expect(span).toEqual({ from, to })
      expect(scalePosition(tokens, span)).toBeCloseTo(needle, 3)
      expect(trailCount(tokens)).toBe(trail)
      expect(meterNote(tokens, 1)).toEqual({ toGo, next, passed })
    })
  }

  it('keeps the needle on the ruler for readings outside the stretch', () => {
    const span = scaleSpan(366_000)
    expect(scalePosition(120_000, span)).toBe(0)
    expect(scalePosition(900_000, span)).toBe(1)
  })

  it('rules about 25 round steps, every fifth longer', () => {
    const ticks = scaleTicks(scaleSpan(366_000))
    expect(ticks).toHaveLength(26)
    expect(ticks[0]).toEqual({ value: 250_000, major: true })
    expect(
      ticks.filter((tick) => tick.major).map((tick) => tick.value),
    ).toEqual([250_000, 300_000, 350_000, 400_000, 450_000, 500_000])
    expect(
      scaleTicks(scaleSpan(6_000))
        .filter((tick) => tick.major)
        .map((tick) => tick.value),
    ).toEqual([0, 2_500, 5_000, 7_500, 10_000])
    expect(
      scaleTicks(scaleSpan(12_000_000))
        .filter((tick) => tick.major)
        .map((tick) => tick.value),
    ).toEqual([10_000_000, 15_000_000, 20_000_000, 25_000_000])
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
