import { describe, expect, it } from 'bun:test'
import type { Plan, SessionSummary } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { MemoryRouter } from 'react-router'
import { usePlan } from '@/modules/api/plan.hooks'
import { ProfileNote } from './ProfileNote'
import { TaskPageHeader } from './TaskPageHeader'

const session: SessionSummary = {
  sessionId: 'profile-session',
  slug: 'claude-code',
  label: 'Claude Code',
  name: 'Order the team’s printer paper',
  startedAt: 1_000,
  durationMs: 60_000,
  dispatchCount: 2,
  toolSequence: ['navigate', 'tabs'],
  status: 'done',
  errorCount: 0,
}

const plan = (status: Plan['status']): Plan => ({
  available: true,
  plan: 'ultrafast',
  planName: 'ultrafast',
  features: [],
  status,
  periodEnd: null,
  endsAt: null,
  graceUntil: null,
  onKey: true,
  checkedAt: null,
  stale: false,
})

function render(node: React.ReactNode, current?: Plan) {
  const queryClient = new QueryClient()
  if (current) queryClient.setQueryData(usePlan.getKey(), current)
  const html = renderToStaticMarkup(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>{node}</MemoryRouter>
    </QueryClientProvider>,
  )
  queryClient.clear()
  return html.replace(/<!-- -->/g, '')
}

describe('task page and browser profiles', () => {
  it('names the profile a task worked in', () => {
    const html = render(
      <TaskPageHeader
        session={{ ...session, browserProfile: { name: 'Work' } }}
      />,
    )
    expect(html).toContain('In your Work profile')
  })

  it('says nothing about profiles for a task in the Default profile', () => {
    expect(render(<ProfileNote session={session} />)).toBe('')
    expect(render(<TaskPageHeader session={session} />)).not.toContain(
      'profile',
    )
  })

  it('tells a Free user the task stayed in Default and where ultrafast is', () => {
    const html = render(
      <ProfileNote session={{ ...session, profileLocked: true }} />,
    )
    expect(html).toContain(
      'Stayed in your Default profile. Choosing a profile comes with ultrafast.',
    )
    expect(html).toContain('See ultrafast')
  })

  it('explains a stop on a paused plan and points to the card', () => {
    const html = render(
      <ProfileNote
        session={{
          ...session,
          status: 'failed',
          stopReason: 'profileNeedsUltrafast',
          browserProfile: { name: 'Work' },
        }}
      />,
      plan('unpaid'),
    )
    expect(html).toContain('This task stopped.')
    expect(html).toContain(
      'It was set to work in your Work profile, and choosing a profile comes with ultrafast. ultrafast is paused, so neo stopped instead of switching to another account.',
    )
    expect(html).toContain('Update card')
    expect(html).toContain(
      'Or ask your agent to run it again in your Default profile.',
    )
  })

  it('offers ultrafast when the plan ended rather than paused', () => {
    const html = render(
      <ProfileNote
        session={{
          ...session,
          stopReason: 'profileNeedsUltrafast',
          browserProfile: { name: 'Work' },
        }}
      />,
      plan('none'),
    )
    expect(html).toContain('ultrafast is off, so neo stopped')
    expect(html).toContain('Get ultrafast')
    expect(html).not.toContain('Update card')
  })
})
