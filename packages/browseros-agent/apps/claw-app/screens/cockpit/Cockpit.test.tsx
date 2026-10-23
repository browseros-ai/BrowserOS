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

const liveSession: LiveSessionCardRecord = {
  sessionId: 'session-connected',
  slug: 'claude-code',
  label: 'Claude Code',
  name: 'Find a flight',
  color: '#2F6FE0',
  startedAt: 100,
  state: 'idle',
  selectedTab: null,
  browserTabs: [],
  toolCount: 0,
  recentTools: [],
}

describe('Cockpit', () => {
  it('shows first-run onboarding until an agent is connected or a task exists', () => {
    const firstRun = renderApp({ connections: 'empty', sessions: 'empty' })
    expect(firstRun).toContain('Your turn')
    expect(firstRun).not.toMatch(/Good (morning|afternoon|evening)/)
    expect(statsQueryEnabled()).toBe(false)

    const ready = renderApp({ connections: 'installed', sessions: 'history' })
    expect(ready).toMatch(/Good (morning|afternoon|evening)/)
    expect(statsQueryEnabled()).toBe(true)
  })

  it('waits for both onboarding probes before choosing a shell or loading stats', () => {
    const html = renderApp({ connections: 'pending', sessions: 'empty' })
    expect(html).not.toContain('Your turn')
    expect(statsQueryEnabled()).toBe(false)
  })

  it('shows the live Home for a connected session even before any setup or history', () => {
    const html = renderApp({
      connections: 'empty',
      sessions: 'empty',
      liveSessions: [liveSession],
    })
    expect(html).toContain('Your AI agents are working on one task.')
    expect(html).toContain('data-stop-session="session-connected"')
    expect(html).not.toContain('Your turn')
  })

  it('shows time saved only once neo has finished a task', () => {
    const measured = renderApp({ connections: 'installed', stats: 'measured' })
    expect(measured).toContain('neo has finished 12 tasks for you so far.')

    const none = renderApp({ connections: 'installed', stats: 'unmeasured' })
    expect(none).not.toContain('neo has finished')
  })
})
