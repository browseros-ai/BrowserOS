import { describe, expect, it, mock } from 'bun:test'
import type { CockpitStats } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import * as _auditHooks from '@/modules/api/audit.hooks'
import * as _cockpitStatsHooks from '@/modules/api/cockpit.hooks'
import * as _connectionsHooks from '@/modules/api/connections.hooks'
import * as _cockpitData from './cockpit.data'
import type { LiveSessionCardRecord } from './cockpit.helpers'

const cockpitDataResultKey = '__browserclawCockpitDataResult'
const connectionsHookResultKey = '__browserclawConnectionsHookResult'
const sessionsHookResultKey = '__browserclawSessionsHookResult'
const statsHookOptionsKey = '__browserclawStatsHookOptions'
const statsHookResultKey = '__browserclawStatsHookResult'

function hookState() {
  return globalThis as Record<string, unknown>
}

// Spread real modules in mock.module replacements: Bun's registry is
// process-scoped, so partial replacements can break unrelated imports.
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
  taskScreenshotUrl: (sessionId: string, id: number) =>
    `/api/v1/sessions/${sessionId}/screenshots/${id}`,
  useTaskScreenshotBaseUrl: () => null,
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
  useCockpitStats: (options?: { enabled?: boolean }) => {
    hookState()[statsHookOptionsKey] = options
    return (
      hookState()[statsHookResultKey] ?? {
        data: undefined,
        isPending: true,
        isError: false,
      }
    )
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
    rawTokenSavingsEstimate: 107_600,
    humanTimeSavedMs: 7_500_000,
    sessionCount: 12,
    toolCallCount: 78,
  },
  last30Days: {
    browserClawTokenEstimate: 2_400,
    screenshotFirstTokenEstimate: 20_000,
    rawTokenSavingsEstimate: 17_600,
    humanTimeSavedMs: 1_500_000,
    sessionCount: 3,
    toolCallCount: 18,
  },
  last7Days: zeroWindow,
}

const unmeasuredStats: CockpitStats = {
  hasMeasuredStats: false,
  allTime: zeroWindow,
  last30Days: zeroWindow,
  last7Days: zeroWindow,
}

type ConnectionsState = 'empty' | 'installed' | 'pending'
type SessionsState = 'empty' | 'history'
type StatsState = 'error' | 'loading' | 'measured' | 'unmeasured'

function setCockpitSessions(sessions: LiveSessionCardRecord[]) {
  hookState()[cockpitDataResultKey] = {
    sessions,
    isPending: false,
  }
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
      : state === 'unmeasured'
        ? { data: unmeasuredStats, isPending: false, isError: false }
        : state === 'error'
          ? { data: undefined, isPending: false, isError: true }
          : { data: undefined, isPending: true, isError: false }
}

function statsQueryEnabled(): boolean | undefined {
  return (hookState()[statsHookOptionsKey] as { enabled?: boolean } | undefined)
    ?.enabled
}

function renderApp(
  options: {
    connections?: ConnectionsState
    liveSessions?: LiveSessionCardRecord[]
    sessions?: SessionsState
    stats?: StatsState
  } = {},
): string {
  setCockpitSessions(options.liveSessions ?? [])
  setConnectionsState(options.connections ?? 'empty')
  setSessionsState(options.sessions ?? 'history')
  setStatsState(options.stats ?? 'loading')
  hookState()[statsHookOptionsKey] = undefined

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

describe('Cockpit (v2)', () => {
  it('renders measured idle stats between the hero and recent activity', () => {
    const html = renderApp({ stats: 'measured' })

    const heroIndex = html.indexOf('working on')
    const savedStatsIndex = html.indexOf('Since you started')
    const recentActivityIndex = html.indexOf('Recent activity')
    expect(heroIndex).toBeGreaterThan(-1)
    expect(savedStatsIndex).toBeGreaterThan(heroIndex)
    expect(recentActivityIndex).toBeGreaterThan(savedStatsIndex)
    expect(html).toContain('nothing running')
    expect(html).not.toContain('Running now')
    expect(statsQueryEnabled()).toBe(true)
  })

  it('always renders RunningGrid for a live session', () => {
    const html = renderApp({
      liveSessions: [liveSession('session-live')],
      stats: 'measured',
    })

    expect(html).toContain('Running now')
    expect(html).not.toContain('Since you started')
    expect(statsQueryEnabled()).toBe(false)
  })

  it('keeps first-run and waiting onboarding shells free of saved stats', () => {
    const firstRun = renderApp({
      connections: 'empty',
      sessions: 'empty',
      stats: 'measured',
    })
    expect(firstRun).toContain('You watch. Your agent')
    expect(firstRun).toContain('Set up MCP endpoint')
    expect(firstRun).toContain(
      'https://cdn.browseros.com/artifacts/claw/onboarding-video/v0.2.0/first-run-demo.mp4',
    )
    expect(firstRun).not.toContain('Since you started')
    expect(statsQueryEnabled()).toBe(false)

    const waiting = renderApp({
      connections: 'installed',
      sessions: 'empty',
      stats: 'measured',
    })
    expect(waiting).toContain('Waiting for your first run')
    expect(waiting).toContain('View MCP endpoint')
    expect(waiting).not.toContain('Since you started')
    expect(statsQueryEnabled()).toBe(false)
  })

  for (const stats of ['loading', 'error', 'unmeasured'] as const) {
    it(`preserves Recent activity while idle stats are ${stats}`, () => {
      const html = renderApp({ stats })

      expect(html).toContain('Recent activity')
      expect(html).not.toContain('Since you started')
      expect(html).not.toContain('Running now')
      expect(statsQueryEnabled()).toBe(true)
    })
  }

  it('does not query stats before the onboarding probes resolve', () => {
    const html = renderApp({
      connections: 'pending',
      sessions: 'empty',
      stats: 'measured',
    })

    expect(html).toContain('Recent activity')
    expect(html).toContain('No recent activity')
    expect(html).not.toContain('Since you started')
    expect(statsQueryEnabled()).toBe(false)
  })

  it('does NOT render an add-profile tile in the default v2 build', () => {
    const html = renderApp()
    expect(html).not.toContain('New profile')
    expect(html).not.toContain('harness . logins . guardrails')
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
    expect(html).not.toContain('Set up MCP endpoint')
    expect(html).not.toContain('Since you started')
    expect(statsQueryEnabled()).toBe(false)
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
