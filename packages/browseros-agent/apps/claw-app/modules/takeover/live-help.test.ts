import { describe, expect, it } from 'bun:test'
import {
  type HelpRequest,
  SessionStatus,
  type SessionSummary,
} from '@browseros/claw-api'
import { takeoverForTab } from './live-help'

function session(over: Partial<SessionSummary> = {}): SessionSummary {
  return {
    sessionId: 'session-1',
    slug: 'claude-code',
    label: 'Claude-code',
    name: 'Outreach',
    durationMs: 0,
    dispatchCount: 3,
    errorCount: 0,
    toolSequence: [],
    status: SessionStatus.Live,
    startedAt: 1_000,
    ...over,
  }
}

function help(over: Partial<HelpRequest> = {}): HelpRequest {
  return {
    requestId: 'help-1',
    reason: 'Enter the login',
    browserTabId: 77,
    requestedAt: 5_000,
    url: 'https://www.netflix.com/login',
    resumeHint: 'play Castlevania',
    ...over,
  }
}

describe('takeoverForTab', () => {
  it('returns the context for the tab a live session is blocked on', () => {
    const sessions = [
      session({
        live: { state: 'idle', browserTabs: [], helpRequest: help() },
      }),
    ]

    const context = takeoverForTab(sessions, 77)

    expect(context).toEqual({
      sessionId: 'session-1',
      agentLabel: 'Claude-code',
      reason: 'Enter the login',
      resumeHint: 'play Castlevania',
      site: 'netflix.com',
      requestedAt: 5_000,
    })
  })

  it('returns null for a tab with no matching help request', () => {
    const sessions = [
      session({
        live: { state: 'idle', browserTabs: [], helpRequest: help() },
      }),
    ]
    expect(takeoverForTab(sessions, 42)).toBeNull()
  })

  it('returns null when no session is waiting for a human', () => {
    expect(takeoverForTab([session()], 77)).toBeNull()
  })

  it('falls back to the page title when the url has no host', () => {
    const sessions = [
      session({
        live: {
          state: 'idle',
          browserTabs: [],
          helpRequest: help({ url: undefined, title: 'Security check' }),
        },
      }),
    ]
    expect(takeoverForTab(sessions, 77)?.site).toBe('Security check')
  })
})
