/**
 * Static-markup checks for the task-centric Audit screen. Stubs the
 * data hook so the test does not need a running backend.
 */

import { describe, expect, it, mock } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { TaskSummary } from '@/modules/api/audit.hooks'
import type { AuditScreenData } from './audit.data'

const baseData: AuditScreenData = {
  tasks: [],
  agentOptions: [],
  statusOptions: [],
  siteOptions: [],
  isLoading: false,
  isError: false,
  hasNextPage: false,
  isFetchingNextPage: false,
  fetchNextPage: () => undefined,
  filters: {
    agentId: null,
    status: null,
    site: null,
    search: '',
    sort: null,
  },
  setAgentFilter: () => undefined,
  setStatusFilter: () => undefined,
  setSiteFilter: () => undefined,
  setSearch: () => undefined,
  setSort: () => undefined,
}

let dataOverride: AuditScreenData = baseData

mock.module('./audit.data', () => ({
  useAuditScreenData: () => dataOverride,
}))

const { Audit } = await import('./Audit')

function renderApp(): string {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <MemoryRouter>
        <Audit />
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

describe('Audit screen', () => {
  it('renders the header', () => {
    dataOverride = { ...baseData }
    const html = renderApp()
    expect(html).toContain('Audit')
  })

  it('shows the editorial empty state when there are no tasks', () => {
    dataOverride = { ...baseData }
    const html = renderApp()
    // Editorial voice: `the audit is *quiet* so far`
    expect(html).toContain('quiet')
  })

  it('shows skeleton loading rows while the first page is pending', () => {
    dataOverride = { ...baseData, isLoading: true }
    const html = renderApp()
    // shadcn Skeleton renders a div with animate-pulse
    expect(html).toMatch(/animate-pulse/)
  })

  it('shows the error empty state when the query fails', () => {
    dataOverride = { ...baseData, isError: true }
    const html = renderApp()
    // Editorial voice: `could not *load* the audit.`
    expect(html).toContain('could not')
  })

  it('renders one row per task with title + agent + site', () => {
    dataOverride = {
      ...baseData,
      tasks: [sampleTask],
      agentOptions: [
        {
          agentId: 'claude-code',
          slug: 'claude-code',
          agentLabel: 'Claude Code',
          count: 1,
        },
      ],
      statusOptions: [{ status: 'done', count: 1 }],
      siteOptions: [{ site: 'example.com', count: 1 }],
    }
    const html = renderApp()
    expect(html).toContain('Claude Code')
    expect(html).toContain('Browsed example.com')
    // DONE is the silent default in the editorial cockpit; the row's
    // identity carries state (LIVE / FAILED render inline dots), so
    // no visible 'Done' text renders here anymore.
  })

  it('renders the Load older tasks button when hasNextPage is true', () => {
    dataOverride = {
      ...baseData,
      tasks: [sampleTask],
      hasNextPage: true,
    }
    const html = renderApp()
    expect(html).toContain('Load older tasks')
  })

  it('keeps the FilterBar visible when a filter yields zero results', () => {
    dataOverride = {
      ...baseData,
      tasks: [],
      filters: {
        agentId: null,
        status: null,
        site: null,
        search: 'nothing-matches',
        sort: null,
      },
    }
    const html = renderApp()
    // FilterBar's search input still on screen so the user can clear /
    // edit their query without a soft-lock.
    expect(html).toMatch(/placeholder="search sessions/)
    // Editorial voice: `nothing *matches* these filters.`
    expect(html).toContain('matches')
  })

  it('hides the FilterBar when there are no tasks AND no active filters', () => {
    dataOverride = { ...baseData, tasks: [] }
    const html = renderApp()
    expect(html).not.toMatch(/placeholder="search sessions/)
    // Editorial voice: `the audit is *quiet* so far`
    expect(html).toContain('quiet')
  })

  it('shows skeleton (not empty state) when auto-paginating a filtered query', () => {
    // The data hook reports isLoading=true while auto-paginating
    // through SQL pages looking for filter matches; the screen must
    // show skeleton, not "No tasks match" prematurely.
    dataOverride = {
      ...baseData,
      isLoading: true,
      tasks: [],
      filters: {
        agentId: null,
        status: 'live',
        site: null,
        search: '',
        sort: null,
      },
    }
    const html = renderApp()
    expect(html).toMatch(/animate-pulse/)
    // Editorial empty states must not appear while data is loading.
    expect(html).not.toContain('quiet so far')
    expect(html).not.toContain('nothing')
  })
})
