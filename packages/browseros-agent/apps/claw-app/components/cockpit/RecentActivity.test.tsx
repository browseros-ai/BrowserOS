import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import * as _auditHooks from '@/modules/api/audit.hooks'

interface MockQueryShape {
  data?: { pages: { items: TaskSummary[] }[] }
  isPending: boolean
}

let queryOverride: MockQueryShape = { isPending: true }
let screenshotBaseUrl: string | null = null

// Spread the real audit-hooks module so unrelated tests that import
// useTaskDetail / useDispatches / useAuditCleanupCandidates keep
// working: Bun's mock.module registry is process-scoped and a
// partial replacement drops the un-overridden exports (see the
// 2026-07-17 test reliability audit).
mock.module('@/modules/api/audit.hooks', () => ({
  ..._auditHooks,
  useSessions: () => queryOverride,
  taskScreenshotUrl: (sessionId: string, id: number, baseUrl?: string) =>
    `${baseUrl ?? ''}/api/v1/sessions/${sessionId}/screenshots/${id}`,
  useTaskScreenshotBaseUrl: () => screenshotBaseUrl,
}))

// Bun has no localStorage; the collapse preference needs one that can also be
// made to throw, the way a tab without storage access does.
const COLLAPSED_KEY = 'cockpitRecentActivityCollapsed'
const memoryStore: Record<string, string> = {}
let storageThrows = false

Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: {
    getItem: (key: string) => {
      if (storageThrows) throw new Error('storage access denied')
      return memoryStore[key] ?? null
    },
    setItem: (key: string, value: string) => {
      if (storageThrows) throw new Error('storage access denied')
      memoryStore[key] = value
    },
    removeItem: (key: string) => {
      delete memoryStore[key]
    },
  },
})

const { RecentActivity } = await import('./RecentActivity')
const { AuditHoverPreview } = await import('../audit/AuditHoverPreview')

function render(): string {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <RecentActivity />
      </MemoryRouter>
    </QueryClientProvider>,
  )
}

const sampleTask: TaskSummary = {
  sessionId: 'sess-1',
  slug: 'claude-code',
  label: 'Claude Code',
  name: 'Browsed example.com',
  site: 'example.com',
  startedAt: Date.now() - 12000,
  endedAt: Date.now(),
  durationMs: 12000,
  dispatchCount: 4,
  toolSequence: ['tabs', 'snapshot', 'read', 'screenshot'],
  status: 'done',
  errorCount: 0,
  latestScreenshotId: 7,
}

/** Twelve runs: six as cards, six as list rows. */
function fullFeed(): TaskSummary[] {
  return Array.from({ length: 12 }, (_, index) => ({
    ...sampleTask,
    sessionId: `cyanotype-${index}`,
    name: `Task ${index}`,
    startedAt: sampleTask.startedAt - index,
  }))
}

// Every test starts from a machine that has never toggled the section.
beforeEach(() => {
  storageThrows = false
  delete memoryStore[COLLAPSED_KEY]
})

describe('RecentActivity', () => {
  it('renders skeleton while pending', () => {
    queryOverride = { isPending: true }
    const html = render()
    expect(html).toMatch(/animate-pulse/)
  })

  it('renders the empty state when there are no tasks', () => {
    queryOverride = { isPending: false, data: { pages: [{ items: [] }] } }
    const html = render()
    expect(html).toContain('No recent activity')
  })

  it('renders each task as a compact session card with title, agent, and meta', () => {
    screenshotBaseUrl = null
    queryOverride = {
      isPending: false,
      data: { pages: [{ items: [sampleTask] }] },
    }
    const html = render()
    expect(html).toContain('Browsed example.com')
    expect(html).toContain('Claude Code')
    expect(html).toContain('ph-no-capture')
    // All cards share the calm light caption tone; there is no lead tile.
    expect(html).toContain('data-caption-tone="light"')
    expect(html).not.toContain('data-caption-tone="blue"')
    // Compact meta line carries the dispatch count (Xs · Nt · ago).
    expect(html).toContain('4t')
  })

  it('keeps the saturated blue caption tone for audit hover previews', () => {
    screenshotBaseUrl = null
    const html = renderToStaticMarkup(<AuditHoverPreview task={sampleTask} />)
    expect(html).toContain('data-caption-tone="blue"')
  })

  it('preserves lead and supporting session screenshots with useful alt text', () => {
    screenshotBaseUrl = 'http://127.0.0.1:9200'
    queryOverride = {
      isPending: false,
      data: {
        pages: [
          {
            items: [
              sampleTask,
              {
                ...sampleTask,
                sessionId: 'sess-2',
                label: 'Codex',
                startedAt: sampleTask.startedAt - 1,
              },
            ],
          },
        ],
      },
    }

    const html = render()
    expect(html).toContain('alt="Session preview from Claude Code"')
    expect(html).toContain('alt="Session preview from Codex"')
    expect(html).not.toContain('data-caption-tone="blue"')
    expect(html.match(/data-caption-tone="light"/g)?.length).toBe(2)
    expect(html).toContain(
      'http://127.0.0.1:9200/api/v1/sessions/sess-1/screenshots/7',
    )
  })

  it('renders the section header + view-all CTA in the empty state', () => {
    queryOverride = { isPending: false, data: { pages: [{ items: [] }] } }
    const html = render()
    expect(html).toContain('Recent activity')
    expect(html).toContain('View all activity')
    expect(html).toContain('href="/audit"')
  })

  it('labels stopped sessions in every recent-activity layout slot', () => {
    const tasks = Array.from({ length: 6 }, (_, index) => ({
      ...sampleTask,
      sessionId: `stopped-${index}`,
      startedAt: sampleTask.startedAt - index,
      status: 'cancelled' as const,
    }))
    queryOverride = { isPending: false, data: { pages: [{ items: tasks }] } }
    const html = render()
    expect(html.match(/STOPPED/g)?.length).toBe(6)
  })

  it('renders a compact card grid, then a minimal list for the rest', () => {
    screenshotBaseUrl = null
    const tasks = fullFeed().map((task, index) => ({
      ...task,
      status: index === 0 ? ('live' as const) : ('done' as const),
    }))
    queryOverride = { isPending: false, data: { pages: [{ items: tasks }] } }

    const html = render()
    expect(html).toContain('12 sessions')
    // First six as cards, the rest as minimal list rows.
    expect(html.match(/data-testid="support-tile-/g)?.length).toBe(6)
    expect(html.match(/data-testid="activity-row-/g)?.length).toBe(6)
    expect(html).toContain('data-testid="recent-activity-list"')
    // A minimal list, not the old heavy activity table.
    expect(html).not.toContain('data-testid="recent-activity-table"')
    expect(html).not.toContain('>Tool chain<')
    // The live run still surfaces its LIVE chip.
    expect(html).toContain('>LIVE<')
    // A machine that has never toggled the section starts expanded.
    expect(html).toContain('aria-expanded="true"')
  })

  it('renders the body collapsed when the preference was persisted', () => {
    screenshotBaseUrl = null
    memoryStore[COLLAPSED_KEY] = 'true'
    queryOverride = {
      isPending: false,
      data: { pages: [{ items: fullFeed() }] },
    }

    const html = render()
    // The body is gone...
    expect(html).not.toContain('data-testid="recent-activity-list"')
    expect(html).not.toContain('data-testid="support-tile-')
    // ...but the header, its count, and the way through to /audit remain.
    expect(html).toContain('Recent activity')
    expect(html).toContain('12 sessions')
    expect(html).toContain('View all activity')
    expect(html).toContain('aria-expanded="false"')
    expect(html).toContain('aria-controls="recent-activity-body"')
  })

  it('falls back to expanded when localStorage throws', () => {
    screenshotBaseUrl = null
    storageThrows = true
    queryOverride = {
      isPending: false,
      data: { pages: [{ items: fullFeed() }] },
    }

    const html = render()
    expect(html).toContain('Recent activity')
    expect(html).toContain('data-testid="recent-activity-list"')
    expect(html).toContain('aria-expanded="true"')
  })
})

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

describe('RecentActivity collapse', () => {
  let root: Root
  let container: HTMLElement
  let queryClient: QueryClient

  beforeEach(async () => {
    screenshotBaseUrl = null
    queryOverride = {
      isPending: false,
      data: { pages: [{ items: fullFeed() }] },
    }
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
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
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

  async function mount(): Promise<void> {
    await act(async () =>
      root.render(
        <QueryClientProvider client={queryClient}>
          <MemoryRouter>
            <RecentActivity />
          </MemoryRouter>
        </QueryClientProvider>,
      ),
    )
  }

  /** Throws the component away and builds it again, as a new tab would. */
  async function remount(): Promise<void> {
    await act(async () => root.unmount())
    const { createRoot } = await import('react-dom/client')
    root = createRoot(container)
    await mount()
  }

  function toggle(): HTMLElement {
    const button = container.querySelector(
      '[data-testid="recent-activity-toggle"]',
    )
    if (!button) throw new Error('recent-activity toggle missing')
    return button as unknown as HTMLElement
  }

  async function clickToggle(): Promise<void> {
    await act(async () => {
      toggle().dispatchEvent(new window.Event('click', { bubbles: true }))
    })
  }

  function bodyIsVisible(): boolean {
    return (
      container.querySelector('[data-testid="recent-activity-list"]') !== null
    )
  }

  it('collapses the body on click and leaves the header and audit link', async () => {
    await mount()
    expect(bodyIsVisible()).toBe(true)

    await clickToggle()

    expect(bodyIsVisible()).toBe(false)
    expect(container.querySelector('[data-testid^="support-tile-"]')).toBeNull()
    expect(toggle().getAttribute('aria-expanded')).toBe('false')
    expect(container.textContent).toContain('Recent activity')
    expect(container.textContent).toContain('12 sessions')
    expect(container.textContent).toContain('View all activity')
  })

  it('remembers a collapsed section across a remount', async () => {
    await mount()
    await clickToggle()
    expect(memoryStore[COLLAPSED_KEY]).toBe('true')

    await remount()

    expect(bodyIsVisible()).toBe(false)
    expect(toggle().getAttribute('aria-expanded')).toBe('false')
  })

  it('remembers an expanded section across a remount', async () => {
    memoryStore[COLLAPSED_KEY] = 'true'
    await mount()
    expect(bodyIsVisible()).toBe(false)

    await clickToggle()
    expect(memoryStore[COLLAPSED_KEY]).toBe('false')

    await remount()

    expect(bodyIsVisible()).toBe(true)
    expect(toggle().getAttribute('aria-expanded')).toBe('true')
  })

  it('still toggles when localStorage writes throw', async () => {
    storageThrows = true
    await mount()

    await clickToggle()

    // The tab forgets the choice, but the section still collapses.
    expect(bodyIsVisible()).toBe(false)
    expect(memoryStore[COLLAPSED_KEY]).toBeUndefined()
  })
})
