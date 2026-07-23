import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { SessionBrowserTab, SessionSummary } from '@browseros/claw-api'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import * as _auditHooks from '@/modules/api/audit.hooks'

let liveItems: SessionSummary[] = []

mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  useLiveSessions: () => ({
    data: { items: liveItems },
    isPending: false,
  }),
  useSessions: () => {
    throw new Error('Running now must not use the history query')
  },
}))

const { useCockpitData } = await import('./cockpit.data')

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

function browserTab(
  browserTabId: number,
  lastActivityAt: number,
): SessionBrowserTab {
  return {
    browserTabId,
    url: `https://tab-${browserTabId}.example/`,
    title: `Tab ${browserTabId}`,
    firstActivityAt: lastActivityAt,
    lastActivityAt,
    lastToolName: 'snapshot',
    toolCount: 1,
    recentTools: [{ name: 'snapshot', at: lastActivityAt }],
  }
}

function liveSession(browserTabs: SessionBrowserTab[]): SessionSummary {
  return {
    sessionId: 'session-1',
    slug: 'codex',
    label: 'Codex',
    name: 'Research BrowserClaw',
    startedAt: 100,
    durationMs: 0,
    dispatchCount: browserTabs.length,
    toolSequence: browserTabs.map(() => 'snapshot'),
    status: 'live',
    errorCount: 0,
    live: { state: 'active', browserTabs },
  }
}

function Probe() {
  const { sessions } = useCockpitData()
  return (
    <output data-selected={sessions[0]?.selectedTab?.browserTabId ?? 'none'} />
  )
}

let root: Root
let container: HTMLElement

beforeEach(async () => {
  liveItems = []
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

async function poll(browserTabs: SessionBrowserTab[]): Promise<string | null> {
  liveItems = [liveSession(browserTabs)]
  await act(async () => root.render(<Probe />))
  return (
    container.querySelector('[data-selected]')?.getAttribute('data-selected') ??
    null
  )
}

describe('useCockpitData', () => {
  it('keeps a selected browser tab across live polls and re-elects after removal', async () => {
    expect(await poll([browserTab(7, 100)])).toBe('7')
    expect(await poll([browserTab(7, 100), browserTab(8, 200)])).toBe('7')
    expect(await poll([browserTab(8, 200), browserTab(9, 300)])).toBe('9')
  })
})
