import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { SessionBrowserTab } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act, type ComponentProps, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import type { Root } from 'react-dom/client'
import * as _popover from '@/components/ui/popover'
import * as _auditHooks from '@/modules/api/audit.hooks'
import * as _cancelHooks from '@/modules/api/cancel.hooks'
import * as _focusHooks from '@/modules/api/focus.hooks'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'

const cancelCalls: Array<{ sessionId: string }> = []
const focusCalls: Array<{ browserTabId: number }> = []
const invalidatedQueryKeys: unknown[] = []

mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  // Null base keeps screenshot cards from making real requests under linkedom.
  useApiBaseUrl: () => null,
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

// Keep the tab list always open, portalled to the body as the real one is.
mock.module('@/components/ui/popover', () => ({
  ..._popover,
  Popover: ({ children }: { children?: ReactNode }) => <>{children}</>,
  PopoverTrigger: (props: ComponentProps<'button'>) => <button {...props} />,
  PopoverContent: ({
    align: _align,
    alignOffset: _alignOffset,
    sideOffset: _sideOffset,
    ...props
  }: ComponentProps<'div'> & {
    align?: string
    alignOffset?: number
    sideOffset?: number
  }) =>
    createPortal(<div data-slot="popover-content" {...props} />, document.body),
}))

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

const { RunningGrid } = await import('./RunningGrid')

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
    agentName: 'Codex',
    label: 'Codex',
    name: 'Research BrowserClaw',
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
        <RunningGrid sessions={sessions} />
      </QueryClientProvider>,
    ),
  )
}

describe('RunningGrid', () => {
  it('shows nothing while no agent is working', async () => {
    await render([])

    expect(container.innerHTML).toBe('')
  })

  it('renders and cancels two same-profile sessions by distinct session ids', async () => {
    await render([
      session({ sessionId: 'session-a' }),
      session({ sessionId: 'session-b' }),
    ])

    expect(
      [...container.querySelectorAll('[data-session-card]')].map((card) =>
        card.getAttribute('data-session-card'),
      ),
    ).toEqual(['session-a', 'session-b'])
    expect(container.querySelector('section')?.classList).toContain(
      'ph-no-capture',
    )
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

  it('says Idle, not Working, once the agent stops making tool calls', async () => {
    await render([
      session({ sessionId: 'session-busy' }),
      session({ sessionId: 'session-quiet', state: 'idle' }),
    ])

    const busy = container.querySelector('[data-session-card="session-busy"]')
    const quiet = container.querySelector('[data-session-card="session-quiet"]')
    expect(busy?.textContent).toContain('Working')
    expect(quiet?.textContent).not.toContain('Working')
    expect(quiet?.textContent).toContain('Idle')
    expect(quiet?.textContent).toContain('Last active on Example')
  })

  it('watches the tab the agent is using', async () => {
    const selectedTab = browserTab({ browserTabId: 42 })
    await render([session({ selectedTab, browserTabs: [selectedTab] })])

    const watch = container.querySelector('[data-watch-browser-tab="42"]')
    if (!watch) throw new Error('Watch button missing')
    await act(async () => {
      watch.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    expect(focusCalls).toEqual([{ browserTabId: 42 }])
  })

  it('counts the tabs only when the agent has more than one', async () => {
    const tabs = [
      browserTab({ browserTabId: 1 }),
      browserTab({ browserTabId: 2 }),
      browserTab({ browserTabId: 3 }),
    ]
    await render([
      session({ sessionId: 'one-tab' }),
      session({ sessionId: 'three-tabs', browserTabs: tabs }),
    ])

    const count = (sessionId: string) =>
      container
        .querySelector(`[data-session-card="${sessionId}"]`)
        ?.querySelector('[data-tab-count]')
    expect(count('one-tab')).toBeNull()
    expect(count('three-tabs')?.textContent).toBe('3 tabs')
  })

  it('goes to the tab picked from the list', async () => {
    const tabs = [
      browserTab({ browserTabId: 1, title: 'Messaging | LinkedIn' }),
      browserTab({ browserTabId: 2, title: 'Sales Navigator' }),
    ]
    await render([session({ browserTabs: tabs })])

    const row = document.querySelector('[data-go-to-browser-tab="2"]')
    if (!row) throw new Error('Sales Navigator row missing')
    await act(async () => {
      row.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    expect(focusCalls).toEqual([{ browserTabId: 2 }])
  })

  it('names each tab and marks the one on the preview', async () => {
    const tabs = [
      browserTab({ browserTabId: 1, title: 'Messaging | LinkedIn' }),
      browserTab({
        browserTabId: 2,
        title: '',
        url: 'https://news.ycombinator.com/',
      }),
    ]
    await render([session({ browserTabs: tabs })])

    const rows = [...document.querySelectorAll('[data-go-to-browser-tab]')]
    expect(rows.map((row) => row.getAttribute('aria-label'))).toEqual([
      'Go to Messaging | LinkedIn',
      'Go to news.ycombinator.com',
    ])
    expect(rows.map((row) => row.getAttribute('aria-current'))).toEqual([
      'true',
      null,
    ])
  })
})
