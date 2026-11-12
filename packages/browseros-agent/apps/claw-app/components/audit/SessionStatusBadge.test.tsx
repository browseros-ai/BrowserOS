import { describe, expect, it } from 'bun:test'
import type {
  LiveSessionActivityState,
  SessionList,
  SessionSummary,
} from '@browseros/claw-api'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderToStaticMarkup } from 'react-dom/server'
import { useLiveSessions } from '@/modules/api/audit.hooks'
import { SessionStatusBadge } from './SessionStatusBadge'

/** A live-session snapshot as the cockpit polls it, holding one session in `state`. */
function liveSnapshot(state: LiveSessionActivityState): SessionList {
  return {
    items: [
      {
        sessionId: 'session-1',
        status: 'live',
        live: { state, browserTabs: [] },
      } as unknown as SessionSummary,
    ],
  } as SessionList
}

function render(
  badge: React.ReactElement,
  snapshot: SessionList | undefined,
): string {
  const client = new QueryClient({
    defaultOptions: { queries: { staleTime: Number.POSITIVE_INFINITY } },
  })
  if (snapshot) client.setQueryData(useLiveSessions.getKey(), snapshot)
  return renderToStaticMarkup(
    <QueryClientProvider client={client}>{badge}</QueryClientProvider>,
  )
}

describe('SessionStatusBadge', () => {
  it('reads a live task Idle when the live snapshot says it has gone quiet', () => {
    const html = render(
      <SessionStatusBadge status="live" sessionId="session-1" />,
      liveSnapshot('idle'),
    )
    expect(html).toContain('Idle')
    expect(html).not.toContain('Working')
  })

  it('keeps Working while the task is active or its activity is unknown', () => {
    expect(
      render(
        <SessionStatusBadge status="live" sessionId="session-1" />,
        liveSnapshot('active'),
      ),
    ).toContain('Working')
    expect(
      render(
        <SessionStatusBadge status="live" sessionId="session-2" />,
        liveSnapshot('idle'),
      ),
    ).toContain('Working')
  })

  it('says how an ended task ended', () => {
    expect(
      render(
        <SessionStatusBadge status="done" sessionId="session-1" />,
        liveSnapshot('idle'),
      ),
    ).toContain('Done')
  })
})
