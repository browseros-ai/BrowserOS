import { describe, expect, it } from 'bun:test'
import type { CockpitStats, CockpitStatsWindow } from '@browseros/claw-api'
import { renderToStaticMarkup } from 'react-dom/server'
import { SavingsCard } from './SavingsCard'

const HOUR_MS = 3_600_000

function window(
  withNeo: number,
  withoutNeo: number,
  humanTimeSavedMs: number,
): CockpitStatsWindow {
  return {
    browserClawTokenEstimate: withNeo,
    screenshotFirstTokenEstimate: withoutNeo,
    rawTokenSavingsEstimate: withoutNeo - withNeo,
    humanTimeSavedMs,
    sessionCount: 3_576,
    toolCallCount: 183_180,
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

// The visible words, without the tags and the `<!-- -->` React puts between text nodes.
function strip(cockpitStats: CockpitStats): string {
  return renderToStaticMarkup(<SavingsCard stats={cockpitStats} />)
    .replace(/<!-- -->/g, '')
    .replace(/<[^>]+>/g, ' ')
    .replace(/\s+/g, ' ')
}

describe('SavingsCard', () => {
  it('shows hours and tokens saved on one line, with saved filling the bar from the left', () => {
    const text = strip(stats(window(314_600_000, 573_900_000, 1_054 * HOUR_MS)))
    expect(text).toContain('1,054 hours saved')
    expect(text).toContain('259.3M AI tokens saved · 45% fewer')
    expect(text).toContain('Details')
    expect(text.indexOf('saved 259.3M')).toBeGreaterThan(-1)
    expect(text.indexOf('saved 259.3M')).toBeLessThan(
      text.indexOf('neo used 314.6M'),
    )
    expect(text).not.toMatch(/without neo/i)
  })

  it('shows hours alone until tokens are measured, and an invitation before the first minute', () => {
    const unmeasured = strip(
      stats(window(314_600_000, 573_900_000, 1_054 * HOUR_MS), false),
    )
    expect(unmeasured).toContain('hours saved')
    expect(unmeasured).not.toContain('AI tokens')
    expect(unmeasured).not.toContain('neo used')

    const firstMinute = strip(stats(window(1, 5, 1_000), false))
    expect(firstMinute).toContain('Your time saved starts here')
    expect(firstMinute).not.toContain('AI tokens')
  })
})
