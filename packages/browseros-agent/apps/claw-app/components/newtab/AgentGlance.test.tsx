import { describe, expect, it } from 'bun:test'
import type { CockpitStats } from '@browseros/claw-api'
import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import { AgentGlance } from './AgentGlance'

const zeroWindow = {
  browserClawTokenEstimate: 0,
  screenshotFirstTokenEstimate: 0,
  rawTokenSavingsEstimate: 0,
  humanTimeSavedMs: 0,
  sessionCount: 0,
  toolCallCount: 0,
}

const measuredStats: CockpitStats = {
  hasMeasuredStats: true,
  allTime: {
    browserClawTokenEstimate: 59_900,
    screenshotFirstTokenEstimate: 105_000,
    rawTokenSavingsEstimate: 45_100,
    humanTimeSavedMs: 2_160_000,
    sessionCount: 9,
    toolCallCount: 54,
  },
  last30Days: zeroWindow,
  last7Days: zeroWindow,
}

const unmeasuredStats: CockpitStats = {
  hasMeasuredStats: false,
  allTime: zeroWindow,
  last30Days: zeroWindow,
  last7Days: zeroWindow,
}

const recentTask: TaskSummary = {
  sessionId: 'sess-1',
  slug: 'claude-code',
  label: 'Claude Code',
  name: 'Read the docs',
  site: 'news.kagi.com',
  startedAt: Date.now() - 60_000,
  endedAt: Date.now(),
  durationMs: 60_000,
  dispatchCount: 3,
  toolSequence: ['navigate', 'snapshot'],
  status: 'done',
  errorCount: 0,
}

function render(node: ReactNode): string {
  return renderToStaticMarkup(<MemoryRouter>{node}</MemoryRouter>)
}

describe('AgentGlance', () => {
  it('summarises the value stat and peeks at recent runs when idle', () => {
    const html = render(
      <AgentGlance
        stats={measuredStats}
        statsPending={false}
        recent={[recentTask]}
        recentPending={false}
      />,
    )

    expect(html).toContain('Idle')
    expect(html).toContain('tokens')
    expect(html).toContain('43% fewer')
    expect(html).toContain('Claude Code')
    expect(html).toContain('news.kagi.com')
    expect(html).toContain('stats')
    expect(html).toContain('all activity')
  })

  it('shows skeletons while stats and recent runs load', () => {
    const html = render(
      <AgentGlance stats={undefined} statsPending recent={[]} recentPending />,
    )

    expect(html).toMatch(/animate-pulse/)
    expect(html).not.toContain('No agent stats yet')
    expect(html).not.toContain('No activity yet')
  })

  it('handles no measured stats and no recent activity', () => {
    const html = render(
      <AgentGlance
        stats={unmeasuredStats}
        statsPending={false}
        recent={[]}
        recentPending={false}
      />,
    )

    expect(html).toContain('No agent stats yet')
    expect(html).toContain('No activity yet')
  })
})
