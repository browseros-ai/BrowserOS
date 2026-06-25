/**
 * Static-markup checks for the Audit screen. Stubs the data hook so
 * the test does not need a running backend.
 */

import { describe, expect, it, mock } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { AuditScreenData } from './audit.data'

const baseData: AuditScreenData = {
  rows: [],
  chips: [],
  isLoading: false,
  isError: false,
  hasNextPage: false,
  isFetchingNextPage: false,
  fetchNextPage: () => undefined,
  selectedAgentId: null,
  setSelectedAgentId: () => undefined,
  now: Date.now(),
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

describe('Audit screen', () => {
  it('renders the header and hint', () => {
    dataOverride = { ...baseData }
    const html = renderApp()
    expect(html).toContain('Audit')
    expect(html).toContain('Every successful tool dispatch')
  })

  it('shows the empty state when there are no dispatches', () => {
    dataOverride = { ...baseData }
    const html = renderApp()
    expect(html).toContain('No dispatches yet')
  })

  it('shows the loading spinner while the first page is pending', () => {
    dataOverride = { ...baseData, isLoading: true }
    const html = renderApp()
    expect(html).toMatch(/<svg|Spinner/)
  })

  it('shows the error empty state when the query fails', () => {
    dataOverride = { ...baseData, isError: true }
    const html = renderApp()
    expect(html).toContain('Could not load audit log')
  })

  it('renders one row per dispatch and the agent filter pill', () => {
    dataOverride = {
      ...baseData,
      rows: [
        {
          id: 1,
          createdAt: Date.now(),
          agentId: 'claude-code',
          slug: 'claude-code',
          agentLabel: 'claude-code',
          sessionId: 'sess-1',
          toolName: 'navigate',
          pageId: 1,
          targetId: 't1',
          url: 'https://example.com',
          title: 'Example',
          argsJson: '{}',
          resultMeta:
            '{"isError":false,"contentSummary":"1 block(s)","structuredKeys":[]}',
          durationMs: 12,
        },
      ],
      chips: [
        {
          agentId: 'claude-code',
          slug: 'claude-code',
          agentLabel: 'claude-code',
          color: '#F26B2A',
          count: 1,
        },
      ],
    }
    const html = renderApp()
    expect(html).toContain('claude-code')
    expect(html).toContain('navigate')
    expect(html).toContain('example.com')
  })

  it('renders the Load older dispatches button when hasNextPage is true', () => {
    dataOverride = {
      ...baseData,
      hasNextPage: true,
      rows: [
        {
          id: 1,
          createdAt: Date.now(),
          agentId: 'a',
          slug: 'a',
          agentLabel: 'a',
          sessionId: 's',
          toolName: 'navigate',
          pageId: null,
          targetId: null,
          url: null,
          title: null,
          argsJson: null,
          resultMeta: null,
          durationMs: null,
        },
      ],
    }
    const html = renderApp()
    expect(html).toContain('Load older dispatches')
  })
})
