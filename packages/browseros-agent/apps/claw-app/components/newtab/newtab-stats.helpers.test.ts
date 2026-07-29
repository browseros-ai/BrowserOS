import { describe, expect, it } from 'bun:test'
import type { CockpitStatsWindow } from '@browseros/claw-api'
import { formatStatGlance } from './newtab-stats.helpers'

function window(over: Partial<CockpitStatsWindow> = {}): CockpitStatsWindow {
  return {
    browserClawTokenEstimate: 12_400,
    screenshotFirstTokenEstimate: 120_000,
    rawTokenSavingsEstimate: 107_600,
    humanTimeSavedMs: 7_500_000,
    sessionCount: 12,
    toolCallCount: 78,
    ...over,
  }
}

describe('formatStatGlance', () => {
  it('compacts tokens saved, formats time saved, and derives percent fewer', () => {
    const glance = formatStatGlance(window())
    expect(glance.tokensSaved).toBe('107.6K')
    expect(glance.timeSaved).toBe('2h 05m')
    expect(glance.percentFewer).toBe('90%')
  })

  it('formats sub-hour time saved in minutes only', () => {
    expect(
      formatStatGlance(window({ humanTimeSavedMs: 36 * 60_000 })).timeSaved,
    ).toBe('36m')
  })

  it('guards divide-by-zero when there is no screenshot-first baseline', () => {
    const glance = formatStatGlance(
      window({ screenshotFirstTokenEstimate: 0, rawTokenSavingsEstimate: 0 }),
    )
    expect(glance.percentFewer).toBe('0%')
  })

  it('never reports negative savings or percent', () => {
    const glance = formatStatGlance(
      window({
        rawTokenSavingsEstimate: -50,
        browserClawTokenEstimate: 200_000,
        screenshotFirstTokenEstimate: 120_000,
      }),
    )
    expect(glance.tokensSaved).toBe('0')
    expect(glance.percentFewer).toBe('0%')
  })
})
