/**
 * Pins the v2 homepage's section set: hero, running grid, recent
 * activity. Legacy waiting and new-profile surfaces must not render
 * in the default v2 build.
 */

import { describe, expect, it, mock } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'

// Stub the data hook so the test does not need a network mock or
// real polling. The shape mirrors the v2 CockpitData interface.
mock.module('./cockpit.data', () => ({
  useCockpitData: () => ({
    agents: [],
    activity: [],
    isPending: false,
  }),
}))

// RecentActivity now consumes useTasks directly. Stub it to return an
// empty page so the empty-state branch renders.
mock.module('@/modules/api/audit.hooks', () => ({
  useTasks: () => ({
    data: { pages: [{ tasks: [], nextCursor: null }] },
    isPending: false,
  }),
  taskScreenshotUrl: (id: number) => `/audit/screenshot/${id}`,
  useTaskScreenshotBaseUrl: () => null,
}))

const { Cockpit } = await import('./Cockpit')

function renderApp(): string {
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
  it('renders the hero and activity header (running grid hides when no agents)', () => {
    const html = renderApp()
    expect(html).toContain('working on')
    expect(html).toContain('Recent activity')
    // No agents in the stub data means RunningGrid returns null.
    expect(html).not.toContain('Running now')
  })

  it('does NOT render the WaitingStrip in the default v2 build', () => {
    const html = renderApp()
    expect(html).not.toContain('Waiting on you')
  })

  it('does NOT render an add-profile tile in the default v2 build', () => {
    const html = renderApp()
    expect(html).not.toContain('New profile')
    expect(html).not.toContain('harness . logins . guardrails')
  })

  it('shows only the recent-activity empty state when registry is empty (Running now hides)', () => {
    const html = renderApp()
    expect(html).not.toContain('No agents connected')
    expect(html).not.toContain('Running now')
    expect(html).toContain('No recent activity')
  })
})
