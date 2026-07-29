import { describe, expect, it } from 'bun:test'
import type { CockpitStats } from '@browseros/claw-api'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { StatGlance } from './StatGlance'

const zeroWindow = {
  browserClawTokenEstimate: 0,
  screenshotFirstTokenEstimate: 0,
  rawTokenSavingsEstimate: 0,
  humanTimeSavedMs: 0,
  sessionCount: 0,
  toolCallCount: 0,
}

const measured: CockpitStats = {
  hasMeasuredStats: true,
  allTime: {
    browserClawTokenEstimate: 12_400,
    screenshotFirstTokenEstimate: 120_000,
    rawTokenSavingsEstimate: 45_100,
    humanTimeSavedMs: 2_160_000,
    sessionCount: 12,
    toolCallCount: 78,
  },
  last30Days: zeroWindow,
  last7Days: zeroWindow,
}

const unmeasured: CockpitStats = {
  hasMeasuredStats: false,
  allTime: zeroWindow,
  last30Days: zeroWindow,
  last7Days: zeroWindow,
}

function render(node: React.ReactNode): string {
  return renderToStaticMarkup(<MemoryRouter>{node}</MemoryRouter>)
}

describe('StatGlance', () => {
  it('renders the full value line with a link to the stats detail', () => {
    const html = render(<StatGlance stats={measured} />)
    expect(html).toContain('45.1K')
    expect(html).toContain('tokens saved')
    expect(html).toContain('36m')
    expect(html).toContain('fewer')
    expect(html).toContain('href="/audit"')
  })

  it('collapses to tokens saved in compact mode', () => {
    const html = render(<StatGlance compact stats={measured} />)
    expect(html).toContain('45.1K saved')
    expect(html).not.toContain('tokens saved')
  })

  it('renders nothing when there are no measured stats', () => {
    expect(render(<StatGlance stats={unmeasured} />)).toBe('')
  })

  it('renders a skeleton while pending', () => {
    const html = render(<StatGlance isPending />)
    expect(html).toMatch(/animate-pulse/)
  })
})
