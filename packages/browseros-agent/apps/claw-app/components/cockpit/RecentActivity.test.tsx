import { describe, expect, it, mock } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
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

  it('renders the freshest task as the lead tile with title, agent, and meta', () => {
    screenshotBaseUrl = null
    queryOverride = {
      isPending: false,
      data: { pages: [{ items: [sampleTask] }] },
    }
    const html = render()
    expect(html).toContain('Browsed example.com')
    expect(html).toContain('Claude Code')
    expect(html).toContain('ph-no-capture')
    expect(html).toContain('data-caption-tone="blue"')
    // DONE is the silent default in the editorial cockpit; the tile
    // instead carries a compact meta line with the dispatch count.
    expect(html).toContain('4 tools')
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
    expect(html).toContain('alt="Session hero from Claude Code"')
    expect(html).toContain('alt="Session preview from Codex"')
    expect(html.match(/data-caption-tone="blue"/g)?.length).toBe(1)
    expect(html.match(/data-caption-tone="light"/g)?.length).toBe(1)
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

  it('renders the cyanotype activity table and counts all visible sessions', () => {
    screenshotBaseUrl = null
    const tasks = Array.from({ length: 12 }, (_, index) => ({
      ...sampleTask,
      sessionId: `cyanotype-${index}`,
      name: `Task ${index}`,
      startedAt: sampleTask.startedAt - index,
      status: index === 0 ? ('live' as const) : ('done' as const),
    }))
    queryOverride = { isPending: false, data: { pages: [{ items: tasks }] } }

    const html = render()
    expect(html).toContain('12 sessions')
    expect(html).toContain('data-testid="recent-activity-table"')
    expect(html).toContain('>Tool chain<')
    expect(html.match(/data-testid="run-row-/g)?.length).toBe(7)
  })
})
