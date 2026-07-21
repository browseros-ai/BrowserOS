import { describe, expect, it, mock } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import * as _auditHooks from '@/modules/api/audit.hooks'
import * as _connectionsHooks from '@/modules/api/connections.hooks'
import * as _cockpitData from './cockpit.data'
import type { LiveSessionCardRecord } from './cockpit.helpers'

// Spread the real module in every mock.module: Bun's registry is
// process-scoped so a partial replacement drops the un-overridden
// exports and breaks unrelated test files that import them (see the
// 2026-07-17 test reliability audit).
mock.module('./cockpit.data', () => ({
  ..._cockpitData,
  useCockpitData: () =>
    cockpitDataHookState()[cockpitDataResultKey] ?? {
      sessions: [],
      isPending: false,
    },
}))

mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  useSessions: () => ({
    data: { pages: [{ items: [] }] },
    isPending: false,
  }),
  taskScreenshotUrl: (sessionId: string, id: number) =>
    `/api/v1/sessions/${sessionId}/screenshots/${id}`,
  useTaskScreenshotBaseUrl: () => null,
}))

const connectionsHookResultKey = '__browserclawConnectionsHookResult'
const cockpitDataResultKey = '__browserclawCockpitDataResult'

function connectionsHookState() {
  return globalThis as Record<string, unknown>
}

function cockpitDataHookState() {
  return globalThis as Record<string, unknown>
}

function setCockpitSessions(sessions: LiveSessionCardRecord[]) {
  cockpitDataHookState()[cockpitDataResultKey] = {
    sessions,
    isPending: false,
  }
}

function setConnectionsProbePending() {
  connectionsHookState()[connectionsHookResultKey] = {
    data: undefined,
    isPending: true,
    isError: false,
  }
}

function setConnectionsProbeEmpty() {
  connectionsHookState()[connectionsHookResultKey] = {
    data: { items: [] },
    isPending: false,
    isError: false,
  }
}

mock.module('@/modules/api/connections.hooks', () => ({
  ..._connectionsHooks,
  useConnections: Object.assign(
    () =>
      connectionsHookState()[connectionsHookResultKey] ?? {
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

const { Cockpit } = await import('./Cockpit')

function renderApp(
  options: {
    connections?: 'pending' | 'empty'
    liveSessions?: LiveSessionCardRecord[]
  } = {},
): string {
  setCockpitSessions(options.liveSessions ?? [])
  if (options.connections === 'empty') {
    setConnectionsProbeEmpty()
  } else {
    setConnectionsProbePending()
  }

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
  it('renders the hero and activity header when connection probing is pending', () => {
    const html = renderApp()
    expect(html).toContain('working on')
    expect(html).toContain('Recent activity')
    expect(html).not.toContain('Running now')
  })

  it('does NOT render an add-profile tile in the default v2 build', () => {
    const html = renderApp()
    expect(html).not.toContain('New profile')
    expect(html).not.toContain('harness . logins . guardrails')
  })

  it('shows only the recent-activity empty state while connection probing is pending', () => {
    const html = renderApp()
    expect(html).not.toContain('No agents connected')
    expect(html).not.toContain('Running now')
    expect(html).toContain('No recent activity')
  })

  it('shows the first-run shell when there are no connections or runs', () => {
    const html = renderApp({ connections: 'empty' })
    expect(html).toContain('You watch. Your agent')
    expect(html).toContain('Set up MCP endpoint')
    expect(html).toContain(
      'https://cdn.browseros.com/artifacts/claw/onboarding-video/v0.2.0/first-run-demo.mp4',
    )
  })

  it('shows a connected zero-tab live session before configuration or activity', () => {
    const html = renderApp({
      connections: 'empty',
      liveSessions: [
        {
          sessionId: 'session-connected',
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
        },
      ],
    })

    expect(html).toContain('Running now')
    expect(html).toContain('data-session-card="session-connected"')
    expect(html).toContain('data-stop-session="session-connected"')
    expect(html).not.toContain('You watch. Your agent')
    expect(html).not.toContain('Set up MCP endpoint')
  })
})
