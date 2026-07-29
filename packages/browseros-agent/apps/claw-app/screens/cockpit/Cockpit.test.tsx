import { describe, expect, it, mock } from 'bun:test'
import type { CockpitStats } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { LiveSessionCardRecord } from './cockpit.helpers'
import type { NewtabData } from './newtab.data'
import * as _newtabData from './newtab.data'

const newtabDataResultKey = '__browserclawNewtabDataResult'

function hookState() {
  return globalThis as Record<string, unknown>
}

mock.module('./newtab.data', () => ({
  ..._newtabData,
  useNewtabData: () => hookState()[newtabDataResultKey],
}))

const { Cockpit } = await import('./Cockpit')

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

function liveSession(sessionId: string): LiveSessionCardRecord {
  return {
    sessionId,
    slug: 'codex',
    label: 'Codex',
    name: 'Connected session',
    harness: 'Codex',
    color: '#7A5AF8',
    startedAt: 100,
    state: 'idle',
    selectedTab: null,
    browserTabs: [],
    toolCount: 0,
    recentTools: [],
  }
}

function newtabData(over: Partial<NewtabData>): NewtabData {
  return {
    state: 'ready',
    isLive: false,
    sessions: [],
    stats: { data: measuredStats, isPending: false },
    recent: { data: { pages: [{ items: [] }] }, isPending: false },
    topSites: { data: [], isPending: false },
    ...over,
  }
}

function renderApp(data: NewtabData): string {
  hookState()[newtabDataResultKey] = data
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <Cockpit />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

describe('Cockpit', () => {
  it('renders the onboarding shell before the reader is ready', () => {
    const html = renderApp(newtabData({ state: 'first-run' }))
    expect(html).toContain('You watch. Your agent')
    expect(html).toContain('Set up MCP endpoint')
    expect(html).not.toContain('Search the web or type a URL')
  })

  it('renders the idle dock with the search focal, the stat, and the recent peek', () => {
    const html = renderApp(newtabData({}))
    expect(html).toContain('Search the web or type a URL')
    expect(html).toContain('No agents running')
    expect(html).toContain('45.1K')
    expect(html).toContain('tokens saved')
    expect(html).toContain('No activity yet')
    expect(html).not.toContain('Running now')
  })

  it('expands into the running view while keeping the search focal anchored', () => {
    const html = renderApp(
      newtabData({ isLive: true, sessions: [liveSession('session-live')] }),
    )
    expect(html).toContain('Search the web or type a URL')
    expect(html).toContain('Running now')
    expect(html).toContain('1 running')
    expect(html).toContain('data-session-card="session-live"')
    expect(html).toContain('45.1K saved')
    expect(html).not.toContain('No agents running')
  })
})
