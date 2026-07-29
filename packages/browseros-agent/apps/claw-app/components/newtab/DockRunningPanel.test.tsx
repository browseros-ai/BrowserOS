import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { SessionBrowserTab } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router'
import * as _auditHooks from '@/modules/api/audit.hooks'
import * as _cancelHooks from '@/modules/api/cancel.hooks'
import * as _focusHooks from '@/modules/api/focus.hooks'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import type { StatsSlice } from '@/screens/cockpit/newtab.data'

const cancelCalls: Array<{ sessionId: string }> = []
const focusCalls: Array<{ browserTabId: number }> = []
const invalidatedQueryKeys: unknown[] = []

mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  useSessionPreviewUrl: () => null,
}))

mock.module('@/modules/api/cancel.hooks', () => ({
  ..._cancelHooks,
  useCancelSession: () => ({
    isPending: false,
    variables: undefined,
    mutate: (
      variables: { sessionId: string },
      options?: { onSuccess?: () => void },
    ) => {
      cancelCalls.push(variables)
      options?.onSuccess?.()
    },
  }),
}))

mock.module('@/modules/api/focus.hooks', () => ({
  ..._focusHooks,
  useFocusBrowserTab: () => ({
    isPending: false,
    variables: undefined,
    mutate: (variables: { browserTabId: number }) => focusCalls.push(variables),
  }),
}))

const { DockRunningPanel } = await import('./DockRunningPanel')

const measuredStats: StatsSlice = {
  isPending: false,
  data: {
    hasMeasuredStats: true,
    allTime: {
      browserClawTokenEstimate: 12_400,
      screenshotFirstTokenEstimate: 120_000,
      rawTokenSavingsEstimate: 45_100,
      humanTimeSavedMs: 2_160_000,
      sessionCount: 12,
      toolCallCount: 78,
    },
    last30Days: {
      browserClawTokenEstimate: 0,
      screenshotFirstTokenEstimate: 0,
      rawTokenSavingsEstimate: 0,
      humanTimeSavedMs: 0,
      sessionCount: 0,
      toolCallCount: 0,
    },
    last7Days: {
      browserClawTokenEstimate: 0,
      screenshotFirstTokenEstimate: 0,
      rawTokenSavingsEstimate: 0,
      humanTimeSavedMs: 0,
      sessionCount: 0,
      toolCallCount: 0,
    },
  },
}

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

function browserTab(over: Partial<SessionBrowserTab> = {}): SessionBrowserTab {
  return {
    browserTabId: 101,
    url: 'https://example.com/foo',
    title: 'Example',
    firstActivityAt: 1_000,
    lastActivityAt: 1_000,
    lastToolName: 'navigate',
    toolCount: 1,
    recentTools: [{ name: 'navigate', at: 1_000 }],
    ...over,
  }
}

function session(
  over: Partial<LiveSessionCardRecord> = {},
): LiveSessionCardRecord {
  const selectedTab = browserTab()
  return {
    sessionId: 'session-live',
    profileId: 'profile-shared',
    slug: 'codex',
    label: 'Codex',
    name: 'Research BrowserClaw',
    harness: 'Codex',
    color: '#0254ec',
    startedAt: 100,
    state: 'active',
    selectedTab,
    browserTabs: [selectedTab],
    toolCount: 1,
    recentTools: [{ name: 'navigate', at: 1_000 }],
    ...over,
  }
}

let root: Root
let container: HTMLElement
let queryClient: QueryClient

beforeEach(async () => {
  cancelCalls.length = 0
  focusCalls.length = 0
  invalidatedQueryKeys.length = 0
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
  container = dom.document.getElementById('root') as unknown as HTMLElement
  queryClient = new QueryClient()
  const invalidateQueries = queryClient.invalidateQueries.bind(queryClient)
  queryClient.invalidateQueries = ((
    ...args: Parameters<QueryClient['invalidateQueries']>
  ) => {
    invalidatedQueryKeys.push(args[0]?.queryKey)
    return invalidateQueries(...args)
  }) as QueryClient['invalidateQueries']
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

async function render(sessions: LiveSessionCardRecord[]) {
  await act(async () =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <MemoryRouter>
          <DockRunningPanel sessions={sessions} stats={measuredStats} />
        </MemoryRouter>
      </QueryClientProvider>,
    ),
  )
}

describe('DockRunningPanel', () => {
  it('renders nothing without any live sessions', async () => {
    await render([])
    expect(container.textContent).not.toContain('Running now')
  })

  it('leads with the newest run and keeps the compact stat in the header', async () => {
    await render([
      session({ sessionId: 'session-old', startedAt: 100 }),
      session({ sessionId: 'session-new', startedAt: 200 }),
    ])

    expect(container.textContent).toContain('Running now')
    expect(container.textContent).toContain('2 running')
    expect(container.textContent).toContain('45.1K saved')
    expect(
      [...container.querySelectorAll('[data-session-card]')].map((card) =>
        card.getAttribute('data-session-card'),
      ),
    ).toEqual(['session-new', 'session-old'])
  })

  it('cancels a session and invalidates the live and history caches', async () => {
    await render([
      session({ sessionId: 'session-a', startedAt: 100 }),
      session({ sessionId: 'session-b', startedAt: 200 }),
    ])

    const stopB = container.querySelector('[data-stop-session="session-b"]')
    if (!stopB) throw new Error('session-b Stop button missing')
    await act(async () => {
      stopB.dispatchEvent(new window.Event('click', { bubbles: true }))
    })

    expect(cancelCalls).toEqual([{ sessionId: 'session-b' }])
    expect(invalidatedQueryKeys).toEqual([
      _auditHooks.useLiveSessions.getKey(),
      _auditHooks.useSessions.getKey(),
    ])
  })

  it('watches the exact selected browser tab id', async () => {
    const selectedTab = browserTab({ browserTabId: 42 })
    await render([
      session({ selectedTab, browserTabs: [selectedTab], startedAt: 200 }),
    ])

    const watch = container.querySelector('[data-watch-browser-tab="42"]')
    if (!watch) throw new Error('Watch button missing')
    await act(async () => {
      watch.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    expect(focusCalls).toEqual([{ browserTabId: 42 }])
  })
})
