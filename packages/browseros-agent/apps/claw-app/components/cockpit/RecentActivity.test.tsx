import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import type { ActivityRow } from '@/modules/api/activity.hooks'
import { RecentActivity } from './RecentActivity'

function renderWithRouter(ui: React.ReactNode): string {
  return renderToStaticMarkup(<MemoryRouter>{ui}</MemoryRouter>)
}

function row(over: Partial<ActivityRow> = {}): ActivityRow {
  return {
    id: 'r1',
    agentLabel: 'claude-code',
    color: '#000',
    status: 'running',
    action: 'navigate to example.com',
    site: 'example.com',
    when: 'just now',
    ...over,
  }
}

describe('RecentActivity', () => {
  it('renders the empty-state card when no rows are present', () => {
    const html = renderWithRouter(<RecentActivity rows={[]} />)
    expect(html).toContain('No recent activity')
    expect(html).toContain('Tool calls from connected agents will appear here.')
  })

  it('still renders the section header in the empty state', () => {
    const html = renderWithRouter(<RecentActivity rows={[]} />)
    expect(html).toContain('Recent activity')
    expect(html).toContain('Across all agents')
  })

  it('renders one row per ActivityRow when rows are present', () => {
    const html = renderWithRouter(
      <RecentActivity
        rows={[
          row({ id: 'r1', action: 'first' }),
          row({ id: 'r2', action: 'second' }),
        ]}
      />,
    )
    expect(html).toContain('first')
    expect(html).toContain('second')
    expect(html).not.toContain('No recent activity')
  })

  it('surfaces a flagged count chip when at least one row is flagged', () => {
    const html = renderWithRouter(
      <RecentActivity
        rows={[row({ id: 'r1', status: 'blocked', action: 'blocked-thing' })]}
      />,
    )
    expect(html).toContain('1 flagged')
  })
})
