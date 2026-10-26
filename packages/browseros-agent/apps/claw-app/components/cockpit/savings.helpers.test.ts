import { describe, expect, it } from 'bun:test'
import type { CockpitStats, CockpitStatsWindow } from '@browseros/claw-api'
import {
  barWidth,
  formatTokens,
  timeSavedAmount,
  usageComparison,
} from './savings.helpers'

function window(withNeo: number, withoutNeo: number): CockpitStatsWindow {
  return {
    browserClawTokenEstimate: withNeo,
    screenshotFirstTokenEstimate: withoutNeo,
    rawTokenSavingsEstimate: withoutNeo - withNeo,
    humanTimeSavedMs: 0,
    sessionCount: 1,
    toolCallCount: 1,
  }
}

function stats(allTime: CockpitStatsWindow, measured = true): CockpitStats {
  return {
    hasMeasuredStats: measured,
    allTime,
    last30Days: allTime,
    last7Days: allTime,
  }
}

describe('usageComparison', () => {
  it('compares usage with and without neo', () => {
    const all = window(1_900_000, 8_800_000)
    expect(usageComparison(stats(all), all)).toEqual({
      withNeo: 1_900_000,
      withoutNeo: 8_800_000,
      percentLess: 78,
    })
  })

  it('shows nothing until there is a measured saving', () => {
    const empty = window(0, 0)
    const worse = window(9, 5)
    const unmeasured = window(1, 5)
    expect(usageComparison(stats(empty), empty)).toBeNull()
    expect(usageComparison(stats(worse), worse)).toBeNull()
    expect(usageComparison(stats(unmeasured, false), unmeasured)).toBeNull()
  })
})

describe('barWidth', () => {
  it('scales to the longest bar and keeps tiny ones visible', () => {
    expect(barWidth(1.9, 8.8)).toBe('21.6%')
    expect(barWidth(8.8, 8.8)).toBe('100%')
    expect(barWidth(0.001, 8.8)).toBe('2%')
    expect(barWidth(1, 0)).toBe('0%')
  })
})

describe('formatting', () => {
  it('writes token counts and time saved the short way', () => {
    expect(formatTokens(1_900_000)).toBe('1.9M')
    expect(formatTokens(180_000)).toBe('180K')
    expect(timeSavedAmount(30 * 3_600_000)).toBe('30 hours')
    expect(timeSavedAmount(60_000)).toBe('1 minute')
    expect(timeSavedAmount(1_000)).toBe('Under a minute')
  })
})
