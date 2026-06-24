import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { AgentActivityRecord } from '@/screens/cockpit/cockpit.helpers'
import { RunningGrid } from './RunningGrid'

function renderWithRouter(ui: React.ReactNode): string {
  return renderToStaticMarkup(<MemoryRouter>{ui}</MemoryRouter>)
}

function agent(over: Partial<AgentActivityRecord> = {}): AgentActivityRecord {
  return {
    agentId: 'claude-code',
    slug: 'claude-code',
    agentLabel: 'claude-code',
    harness: null,
    color: '#000',
    status: 'active',
    lastToolAt: 0,
    lastToolName: 'navigate',
    toolCount: 3,
    recentTools: ['navigate', 'read', 'tabs'],
    tabs: [],
    currentFocus: {
      targetId: 't1',
      pageId: 1,
      url: 'https://example.com/foo',
      title: 'Example',
      lastToolAt: 0,
      lastToolName: 'navigate',
      toolCount: 3,
      recentTools: ['navigate'],
    } as AgentActivityRecord['currentFocus'],
    ...over,
  }
}

describe('RunningGrid', () => {
  it('renders the empty-state card when no agents are present', () => {
    const html = renderWithRouter(<RunningGrid agents={[]} />)
    expect(html).toContain('No agents connected')
    expect(html).toContain('MCP page')
    expect(html).toContain('/mcp')
  })

  it('still renders the "Running now · 0 live" header in the empty state', () => {
    const html = renderWithRouter(<RunningGrid agents={[]} />)
    expect(html).toContain('Running now')
    expect(html).toContain('0 live')
  })

  it('does not render the legacy AddAgentTile when agents are present', () => {
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
