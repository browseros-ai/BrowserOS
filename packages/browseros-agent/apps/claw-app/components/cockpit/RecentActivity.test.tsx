import { describe, expect, it, mock } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { TaskSummary } from '@/modules/api/audit.hooks'

interface MockQueryShape {
  data?: { pages: { tasks: TaskSummary[] }[] }
  isPending: boolean
}

let queryOverride: MockQueryShape = { isPending: true }

mock.module('@/modules/api/audit.hooks', () => ({
  useTasks: () => queryOverride,
  taskScreenshotUrl: (id: number) => `/audit/screenshot/${id}`,
}))

const { RecentActivity } = await import('./RecentActivity')

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
  agentId: 'claude-code',
  slug: 'claude-code',
  agentLabel: 'Claude Code',
  title: 'Browsed example.com',
  site: 'example.com',
  startedAt: Date.now() - 12000,
  endedAt: Date.now(),
  durationMs: 12000,
  dispatchCount: 4,
  toolSequence: ['tabs', 'snapshot', 'read', 'screenshot'],
  status: 'done',
  errorCount: 0,
  lastScreenshotDispatchId: 7,
  cursorId: 8,
}

describe('RecentActivity', () => {
  it('renders skeleton while pending', () => {
    queryOverride = { isPending: true }
    const html = render()
    expect(html).toMatch(/animate-pulse/)
  })

  it('renders the empty state when there are no tasks', () => {
    queryOverride = { isPending: false, data: { pages: [{ tasks: [] }] } }
    const html = render()
    expect(html).toContain('No recent activity')
  })

  it('renders the freshest task as the lead tile with title, agent, and meta', () => {
    queryOverride = {
      isPending: false,
      data: { pages: [{ tasks: [sampleTask] }] },
    }
    const html = render()
    expect(html).toContain('Browsed example.com')
    expect(html).toContain('Claude Code')
    // DONE is the silent default in the editorial cockpit; the tile
    // instead carries a mono meta line with the dispatch count.
    expect(html).toContain('4 tools')
  })

  it('renders the section header + view-all CTA in the empty state', () => {
    queryOverride = { isPending: false, data: { pages: [{ tasks: [] }] } }
    const html = render()
    expect(html).toContain('Recent activity')
    expect(html).toContain('View all activity')
    expect(html).toContain('href="/audit"')
  })
})
