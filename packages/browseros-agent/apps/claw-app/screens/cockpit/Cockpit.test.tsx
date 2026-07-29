import { describe, expect, it, mock } from 'bun:test'
import type { CockpitStats } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import * as _topSitesHooks from '@/components/newtab/top-sites.hooks'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import * as _auditHooks from '@/modules/api/audit.hooks'
import * as _cockpitStatsHooks from '@/modules/api/cockpit.hooks'
import * as _connectionsHooks from '@/modules/api/connections.hooks'
import * as _cockpitData from './cockpit.data'
import type { LiveSessionCardRecord } from './cockpit.helpers'

const cockpitDataResultKey = '__browserclawCockpitDataResult'
const connectionsHookResultKey = '__browserclawConnectionsHookResult'
const sessionsHookResultKey = '__browserclawSessionsHookResult'
const statsHookResultKey = '__browserclawStatsHookResult'
const statsHookCallsKey = '__browserclawStatsHookCalls'
const topSitesHookResultKey = '__browserclawTopSitesHookResult'

function hookState() {
  return globalThis as Record<string, unknown>
}

mock.module('./cockpit.data', () => ({
  ..._cockpitData,
  useCockpitData: () =>
    hookState()[cockpitDataResultKey] ?? {
      sessions: [],
      isPending: false,
    },
}))

mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  useSessions: () =>
    hookState()[sessionsHookResultKey] ?? {
      data: { pages: [{ items: [] }] },
      isPending: false,
    },
  useSessionPreviewUrl: () => null,
}))

mock.module('@/modules/api/connections.hooks', () => ({
  ..._connectionsHooks,
  useConnections: Object.assign(
    () =>
      hookState()[connectionsHookResultKey] ?? {
        data: undefined,
        isPending: true,
        isError: false,
      },
    { getKey: () => ['cockpit', 'connections'] },
  ),
  useConnectHarness: () => ({
    isPending: false,
    variables: undefined,
    mutateAsync: async () => ({ installed: true }),
  }),
  useDisconnectHarness: () => ({
    isPending: false,
    variables: undefined,
    mutateAsync: async () => ({ installed: false }),
  }),
}))

mock.module('@/modules/api/cockpit.hooks', () => ({
  ..._cockpitStatsHooks,
  useCockpitStats: () => {
    hookState()[statsHookCallsKey] =
      ((hookState()[statsHookCallsKey] as number) ?? 0) + 1
    return (
      hookState()[statsHookResultKey] ?? {
        data: undefined,
        isPending: true,
        isError: false,
      }
    )
  },
}))

mock.module('@/components/newtab/top-sites.hooks', () => ({
  ..._topSitesHooks,
  useTopSites: () =>
    hookState()[topSitesHookResultKey] ?? {
      data: [],
      isPending: false,
      isError: false,
    },
}))

const { Cockpit } = await import('./Cockpit')

const sampleTask: TaskSummary = {
  sessionId: 'session-history',
  slug: 'codex',
  label: 'Codex',
  name: 'Finished a task',
  site: 'example.com',
  startedAt: 100,
  endedAt: 200,
  durationMs: 100,
  dispatchCount: 2,
  toolSequence: ['snapshot', 'act'],
  status: 'done',
  errorCount: 0,
}

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

type ConnectionsState = 'empty' | 'installed' | 'pending'
type SessionsState = 'empty' | 'history'
type StatsState = 'loading' | 'measured'

function setCockpitSessions(sessions: LiveSessionCardRecord[]) {
  hookState()[cockpitDataResultKey] = { sessions, isPending: false }
}

function setConnectionsState(state: ConnectionsState) {
  hookState()[connectionsHookResultKey] =
    state === 'pending'
      ? { data: undefined, isPending: true, isError: false }
      : {
          data: {
            items:
              state === 'installed'
                ? [
                    {
                      harness: 'Codex',
                      installed: true,
                      message: 'Configured in Codex.',
                    },
                  ]
                : [],
          },
          isPending: false,
          isError: false,
        }
}

function setSessionsState(state: SessionsState) {
  hookState()[sessionsHookResultKey] = {
    data: { pages: [{ items: state === 'history' ? [sampleTask] : [] }] },
    isPending: false,
  }
}

function setStatsState(state: StatsState) {
  hookState()[statsHookResultKey] =
    state === 'measured'
      ? { data: measuredStats, isPending: false, isError: false }
      : { data: undefined, isPending: true, isError: false }
}

function setTopSites(items: Array<{ url: string; title: string }>) {
  hookState()[topSitesHookResultKey] = {
    data: items,
    isPending: false,
    isError: false,
  }
}

function statsCallCount(): number {
  return (hookState()[statsHookCallsKey] as number) ?? 0
}

function renderApp(
  options: {
    connections?: ConnectionsState
    liveSessions?: LiveSessionCardRecord[]
    sessions?: SessionsState
    stats?: StatsState
    topSites?: Array<{ url: string; title: string }>
  } = {},
): string {
  setCockpitSessions(options.liveSessions ?? [])
  setConnectionsState(options.connections ?? 'empty')
  setSessionsState(options.sessions ?? 'history')
  setStatsState(options.stats ?? 'loading')
  setTopSites(options.topSites ?? [])
  hookState()[statsHookCallsKey] = 0

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

describe('Cockpit new-tab monitor', () => {
  it('renders the calm idle page: brand mark, omnibox, top sites, one glance line', () => {
    const html = renderApp({
      stats: 'measured',
      topSites: [{ url: 'https://github.com/', title: 'GitHub' }],
    })

    expect(html).toContain('alt="BrowserClaw"')
    expect(html).toContain('Search the web or type a URL')
    expect(html).toContain('data-top-site="https://github.com/"')
    expect(html).toContain('Idle')
    expect(html).toContain('tokens saved')
    expect(html).not.toContain('Running now')
    expect(statsCallCount()).toBeGreaterThan(0)
  })

  it('promotes the running monitor to the hero for a live session', () => {
    const html = renderApp({
      liveSessions: [liveSession('session-live')],
      stats: 'measured',
    })

    const runningIndex = html.indexOf('Running now')
    const searchIndex = html.indexOf('Search the web or type a URL')
    expect(runningIndex).toBeGreaterThan(-1)
    expect(html).toContain('data-session-card="session-live"')
    // The running block leads the column; search demotes beneath it.
    expect(searchIndex).toBeGreaterThan(runningIndex)
    expect(html).not.toContain('alt="BrowserClaw"')
    expect(statsCallCount()).toBeGreaterThan(0)
  })

  it('keeps onboarding shells free of the monitor and its stats query', () => {
    const firstRun = renderApp({
      connections: 'empty',
      sessions: 'empty',
      stats: 'measured',
    })
    expect(firstRun).toContain('You watch. Your agent')
    expect(firstRun).toContain('Set up MCP endpoint')
    expect(firstRun).not.toContain('Running now')
    expect(firstRun).not.toContain('Search the web or type a URL')
    expect(statsCallCount()).toBe(0)

    const waiting = renderApp({
      connections: 'installed',
      sessions: 'empty',
      stats: 'measured',
    })
    expect(waiting).toContain('Waiting for your first run')
    expect(waiting).not.toContain('Running now')
    expect(statsCallCount()).toBe(0)
  })

  it('shows a connected zero-tab live session before configuration or activity', () => {
    const html = renderApp({
      connections: 'empty',
      liveSessions: [liveSession('session-connected')],
      sessions: 'empty',
      stats: 'measured',
    })

    expect(html).toContain('Running now')
    expect(html).toContain('data-session-card="session-connected"')
    expect(html).toContain('data-stop-session="session-connected"')
    expect(html).not.toContain('You watch. Your agent')
  })
})

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
