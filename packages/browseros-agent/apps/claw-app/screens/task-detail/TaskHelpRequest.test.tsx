import { describe, expect, it } from 'bun:test'
import type { SessionList, SessionSummary } from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { SessionStatusBadge } from '@/components/audit/SessionStatusBadge'
import { useLiveSessions } from '@/modules/api/audit.hooks'
import { TaskHelpRequest } from './TaskHelpRequest'

const session: SessionSummary = {
  sessionId: 'approval-session',
  slug: 'cursor',
  label: 'Cursor',
  name: 'Approve a local request',
  startedAt: 1_000,
  durationMs: 60_000,
  dispatchCount: 1,
  toolSequence: ['run'],
  status: 'live',
  errorCount: 0,
}

const pending: SessionSummary = {
  ...session,
  live: {
    state: 'idle',
    browserTabs: [],
    helpRequest: {
      requestId: 'approval-1',
      reason: 'Please approve this request before I submit it.',
      kind: 'approval',
      browserTabId: 42,
      requestedAt: 2_000,
      resumeHint: 'Submit the approved request.',
    },
  },
}

function renderHelp(
  detailSession = session,
  liveSession: SessionSummary | undefined = pending,
) {
  const queryClient = new QueryClient()
  queryClient.setQueryData<SessionList>(useLiveSessions.getKey(), {
    items: liveSession ? [liveSession] : [],
  })
  const html = renderToStaticMarkup(
    <QueryClientProvider client={queryClient}>
      <SessionStatusBadge
        status={detailSession.status}
        sessionId={detailSession.sessionId}
      />
      <TaskHelpRequest session={detailSession} />
    </QueryClientProvider>,
  )
  queryClient.clear()
  return html
}

describe('task detail pending help', () => {
  it('reads the pending help snapshot for both the badge and Home help controls', () => {
    const html = renderHelp()
    expect(html).toContain('Waiting for you')
    expect(html).toContain(pending.live?.helpRequest?.reason ?? '')
    expect(html).toContain('Help now')
    expect(html).toContain('data-take-over="approval-session"')
    expect(html).toContain('Stop task')
    expect(html).not.toContain('>Idle<')
  })

  it('removes the reason and help controls when the live request clears', () => {
    const html = renderHelp(session, {
      ...session,
      live: { state: 'idle', browserTabs: [] },
    })
    expect(html).toContain('Idle')
    expect(html).not.toContain('Waiting for you')
    expect(html).not.toContain('Help now')
    expect(html).not.toContain(pending.live?.helpRequest?.reason ?? '')
  })

  it('keeps an ended task outcome despite stale pending help in the cache', () => {
    const html = renderHelp({ ...session, status: 'done' })
    expect(html).toContain('Done')
    expect(html).not.toContain('Waiting for you')
    expect(html).not.toContain('Help now')
  })
})
