/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  meterParts,
  meterTokens,
  milestoneNote,
  nextMilestone,
  passedMilestones,
} from './energy-meter.helpers'

describe('the line under the total', () => {
  it('says the first ultrafast run is in', () => {
    expect(milestoneNote(6_118, 1)).toBe('Your first ultrafast run is in')
  })

  it('says a milestone was just passed until halfway to the next, then nothing', () => {
    expect(milestoneNote(253_678, 5)).toBe('You just passed 250k')
    expect(milestoneNote(326_060, 5)).toBe('You just passed 250k')
    expect(milestoneNote(400_000, 7)).toBeNull()
    expect(milestoneNote(101_500, 3)).toBe('You just passed 100k')
    expect(milestoneNote(1_004_000, 9)).toBe('You just passed 1M')
    expect(milestoneNote(2_700_000, 40)).toBe('You just passed 2.5M')
  })

  it('says nothing below the first milestone after more than one run', () => {
    expect(milestoneNote(4_000, 3)).toBeNull()
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
