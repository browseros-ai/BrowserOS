import { describe, expect, it } from 'bun:test'
import type { LiveSessionCardRecord } from './cockpit.helpers'
import { formatElapsed, pickLeadSession } from './newtab.helpers'

function session(
  over: Partial<LiveSessionCardRecord> = {},
): LiveSessionCardRecord {
  return {
    sessionId: 'session-a',
    slug: 'codex',
    label: 'Codex',
    name: 'A task',
    harness: 'Codex',
    color: '#0254ec',
    startedAt: 100,
    state: 'active',
    selectedTab: null,
    browserTabs: [],
    toolCount: 0,
    recentTools: [],
    ...over,
  }
}

describe('pickLeadSession', () => {
  it('returns no lead for an empty list', () => {
    expect(pickLeadSession([])).toEqual({ lead: null, rest: [] })
  })

  it('elects the session with the most recent tool activity as the lead', () => {
    const stale = session({
      sessionId: 'stale',
      recentTools: [{ name: 'navigate', at: 1_000 }],
    })
    const fresh = session({
      sessionId: 'fresh',
      recentTools: [
        { name: 'navigate', at: 1_000 },
        { name: 'click', at: 5_000 },
      ],
    })
    const { lead, rest } = pickLeadSession([stale, fresh])
    expect(lead?.sessionId).toBe('fresh')
    expect(rest.map((s) => s.sessionId)).toEqual(['stale'])
  })

  it('breaks a tool-activity tie by most recent start then session id', () => {
    const older = session({ sessionId: 'b', startedAt: 100, recentTools: [] })
    const newer = session({ sessionId: 'a', startedAt: 900, recentTools: [] })
    expect(pickLeadSession([older, newer]).lead?.sessionId).toBe('a')

    const sameStart = [
      session({ sessionId: 'zeta', startedAt: 100, recentTools: [] }),
      session({ sessionId: 'alpha', startedAt: 100, recentTools: [] }),
    ]
    expect(pickLeadSession(sameStart).lead?.sessionId).toBe('alpha')
  })

  it('is stable regardless of input order', () => {
    const one = session({
      sessionId: 'one',
      recentTools: [{ name: 'a', at: 10 }],
    })
    const two = session({
      sessionId: 'two',
      recentTools: [{ name: 'a', at: 20 }],
    })
    expect(pickLeadSession([one, two]).lead?.sessionId).toBe('two')
    expect(pickLeadSession([two, one]).lead?.sessionId).toBe('two')
  })
})

describe('formatElapsed', () => {
  it('formats seconds, minutes, hours, and days without an "ago" suffix', () => {
    expect(formatElapsed(0, 53_000)).toBe('53s')
    expect(formatElapsed(0, 4 * 60_000)).toBe('4m')
    expect(formatElapsed(0, 2 * 3_600_000)).toBe('2h')
    expect(formatElapsed(0, 3 * 86_400_000)).toBe('3d')
  })

  it('never returns a negative elapsed for a future start', () => {
    expect(formatElapsed(10_000, 0)).toBe('0s')
  })
})
