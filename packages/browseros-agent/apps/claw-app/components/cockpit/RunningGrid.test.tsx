import { describe, expect, it } from 'bun:test'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { TabActivityRecord } from '@/modules/api/tabs.hooks'
import type { AgentActivityRecord } from '@/screens/cockpit/cockpit.helpers'
import { RunningGrid } from './RunningGrid'

function renderWithRouter(ui: React.ReactNode): string {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>
      <MemoryRouter>{ui}</MemoryRouter>
    </QueryClientProvider>,
  )
}

function tab(over: Partial<TabActivityRecord> = {}): TabActivityRecord {
  return {
    targetId: 't1',
    pageId: 1,
    url: 'https://example.com/foo',
    title: 'Example',
    agentId: 'claude-code',
    slug: 'claude-code',
    firstToolAt: 0,
    lastToolAt: 0,
    lastToolName: 'navigate',
    toolCount: 3,
    recentTools: [{ name: 'navigate', at: 0 }],
    status: 'active',
    agentLabel: 'claude-code',
    harness: null,
    color: null,
    screencast: null,
    ...over,
  }
}

function agent(over: Partial<AgentActivityRecord> = {}): AgentActivityRecord {
  const focus = tab()
  return {
    agentId: 'claude-code',
    slug: 'claude-code',
    agentLabel: 'claude-code',
    harness: 'Claude Code',
    color: '#000',
    status: 'active',
    firstToolAt: 0,
    lastToolAt: 0,
    lastToolName: 'navigate',
    toolCount: 3,
    recentTools: [
      { name: 'navigate', at: 0 },
      { name: 'read', at: 0 },
      { name: 'tabs', at: 0 },
    ],
    tabs: [focus],
    currentFocus: focus,
    ...over,
  }
}

describe('RunningGrid', () => {
  it('renders nothing when no agents are connected', () => {
    const html = renderWithRouter(<RunningGrid agents={[]} />)
    expect(html).toBe('')
  })

  it('hides the header entirely in the empty case', () => {
    const html = renderWithRouter(<RunningGrid agents={[]} />)
    expect(html).not.toContain('Running now')
    expect(html).not.toContain('0 live')
  })

  it('does not render an add-profile tile when agents are present', () => {
    const html = renderWithRouter(<RunningGrid agents={[agent()]} />)
    expect(html).not.toContain('New profile')
    expect(html).not.toContain('harness . logins . guardrails')
  })

  it('renders one card per agent and reflects the live count', () => {
    const html = renderWithRouter(
      <RunningGrid
        agents={[agent({ agentId: 'a' }), agent({ agentId: 'b' })]}
      />,
    )
    expect(html).toContain('2 live')
  })
})
