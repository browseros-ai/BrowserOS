/**
 * Pins the v2 homepage's section set: hero, running grid, recent
 * activity. The waiting strip and the new-profile tile must NOT
 * render in the default v2 build (their files stay on disk with TODO
 * headers, but the rendered tree does not reach them).
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
  it('renders the hero, running grid header, and activity header', () => {
    const html = renderApp()
    expect(html).toContain('working on')
    expect(html).toContain('Running now')
    expect(html).toContain('Recent activity')
  })

  it('does NOT render the WaitingStrip in the default v2 build', () => {
    const html = renderApp()
    expect(html).not.toContain('Waiting on you')
  })

  it('does NOT render the AddAgentTile in the default v2 build', () => {
    const html = renderApp()
    expect(html).not.toContain('New profile')
    expect(html).not.toContain('harness . logins . guardrails')
  })

  it('renders both empty-state cards when registry is empty', () => {
    const html = renderApp()
    expect(html).toContain('No agents connected')
    expect(html).toContain('No recent activity')
  })
})
