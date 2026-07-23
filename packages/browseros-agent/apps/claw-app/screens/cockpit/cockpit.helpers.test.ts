import { describe, expect, it } from 'bun:test'
import type { SessionBrowserTab, SessionSummary } from '@browseros/claw-api'
import {
  colorForSlug,
  formatRelative,
  formatToolTrail,
  harnessForRow,
  sessionsToLiveCards,
  siteOf,
} from './cockpit.helpers'

function browserTab(over: Partial<SessionBrowserTab> = {}): SessionBrowserTab {
  return {
    browserTabId: 101,
    url: 'https://example.com/foo',
    title: 'Example',
    firstActivityAt: 1_000,
    lastActivityAt: 1_000,
    lastToolName: 'navigate',
    toolCount: 1,
    recentTools: [{ name: 'navigate', at: 1_000 }],
    ...over,
  }
}

function liveSession(over: Partial<SessionSummary> = {}): SessionSummary {
  return {
    sessionId: 'session-1',
    profileId: 'profile-shared',
    slug: 'codex',
    label: 'Codex',
    name: 'Research BrowserClaw',
    harness: 'Codex',
    color: '#0254ec',
    startedAt: 100,
    durationMs: 10,
    dispatchCount: 1,
    toolSequence: ['navigate'],
    status: 'live',
    errorCount: 0,
    live: { state: 'active', browserTabs: [browserTab()] },
    ...over,
  }
}

describe('siteOf', () => {
  it('returns the host without leading www', () => {
    expect(siteOf('https://www.example.com/foo')).toBe('example.com')
    expect(siteOf('https://docs.google.com/sheets/abc')).toBe('docs.google.com')
  })

  it('falls back to the raw URL for invalid input', () => {
    expect(siteOf('not a url')).toBe('not a url')
  })
})

describe('formatRelative', () => {
  it('formats seconds, minutes, hours, and days', () => {
    expect(formatRelative(95_000, 100_000)).toBe('5s ago')
    expect(formatRelative(0, 60_000)).toBe('1m ago')
    expect(formatRelative(0, 3_600_000)).toBe('1h ago')
    expect(formatRelative(0, 24 * 3_600_000)).toBe('1d ago')
  })
})

describe('display fallbacks', () => {
  it('keeps slug colors deterministic', () => {
    expect(colorForSlug('finance')).toBe(colorForSlug('finance'))
    expect(colorForSlug('travel')).toMatch(/^#[0-9A-F]{6}$/i)
  })

  it('keeps known harnesses and uses the existing fallback', () => {
    expect(harnessForRow('Cursor')).toBe('Cursor')
    expect(harnessForRow('Codex')).toBe('Codex')
    expect(harnessForRow(undefined)).toBe('Claude Code')
    expect(harnessForRow('Atlas-9000')).toBe('Claude Code')
  })
})

describe('formatToolTrail', () => {
  it('joins tool names and caps the visible tail', () => {
    const tools = ['navigate', 'snapshot', 'act', 'read', 'grep', 'screenshot']
    expect(
      formatToolTrail(
        tools.map((name, index) => ({ name, at: index })),
        4,
      ),
    ).toBe('act -> read -> grep -> screenshot')
  })

  it('returns an empty string for an empty trail', () => {
    expect(formatToolTrail([])).toBe('')
  })
})

describe('sessionsToLiveCards', () => {
  it('keeps two same-profile sessions as distinct cards and Stop identities', () => {
    const cards = sessionsToLiveCards([
      liveSession({ sessionId: 'session-a', startedAt: 100 }),
      liveSession({ sessionId: 'session-b', startedAt: 200 }),
    ])

    expect(cards.map((card) => card.sessionId)).toEqual([
      'session-a',
      'session-b',
    ])
    expect(cards.map((card) => card.profileId)).toEqual([
      'profile-shared',
      'profile-shared',
    ])
  })

  it('keeps a dispatch-backed zero-tab live session as an idle card', () => {
    const [card] = sessionsToLiveCards([
      liveSession({
        sessionId: 'session-empty',
        dispatchCount: 1,
        toolSequence: ['name_session'],
        live: { state: 'idle', browserTabs: [] },
      }),
    ])

    expect(card).toMatchObject({
      sessionId: 'session-empty',
      state: 'idle',
      selectedTab: null,
      browserTabs: [],
      toolCount: 0,
      recentTools: [],
    })
  })

  it('merges a multi-tab count and chronological recent-tool trail', () => {
    const [card] = sessionsToLiveCards([
      liveSession({
        live: {
          state: 'active',
          browserTabs: [
            browserTab({
              browserTabId: 101,
              toolCount: 2,
              recentTools: [
                { name: 'navigate', at: 100 },
                { name: 'snapshot', at: 300 },
              ],
            }),
            browserTab({
              browserTabId: 102,
              toolCount: 2,
              recentTools: [
                { name: 'read', at: 200 },
                { name: 'act', at: 400 },
              ],
            }),
          ],
        },
      }),
    ])

    expect(card.browserTabs).toHaveLength(2)
    expect(card.toolCount).toBe(4)
    expect(card.recentTools.map((tool) => tool.name)).toEqual([
      'navigate',
      'read',
      'snapshot',
      'act',
    ])
  })

  it('elects the freshest activity-bearing browser tab', () => {
    const [card] = sessionsToLiveCards([
      liveSession({
        live: {
          state: 'active',
          browserTabs: [
            browserTab({ browserTabId: 101, lastActivityAt: 1_000 }),
            browserTab({ browserTabId: 102, lastActivityAt: 2_000 }),
            browserTab({
              browserTabId: 100,
              firstActivityAt: undefined,
              lastActivityAt: undefined,
            }),
          ],
        },
      }),
    ])

    expect(card.selectedTab?.browserTabId).toBe(102)
  })

  it('falls back deterministically by browser tab id without activity', () => {
    const [card] = sessionsToLiveCards([
      liveSession({
        live: {
          state: 'idle',
          browserTabs: [
            browserTab({
              browserTabId: 202,
              firstActivityAt: undefined,
              lastActivityAt: undefined,
            }),
            browserTab({
              browserTabId: 201,
              firstActivityAt: undefined,
              lastActivityAt: undefined,
            }),
          ],
        },
      }),
    ])

    expect(card.selectedTab?.browserTabId).toBe(201)
  })

  it('keeps a sticky browser tab while present, then re-elects when it disappears', () => {
    const initial = liveSession({
      live: {
        state: 'active',
        browserTabs: [
          browserTab({ browserTabId: 101, lastActivityAt: 1_000 }),
          browserTab({ browserTabId: 102, lastActivityAt: 2_000 }),
        ],
      },
    })
    const [sticky] = sessionsToLiveCards([initial], {
      stickySelection: new Map([['session-1', 101]]),
    })
    expect(sticky.selectedTab?.browserTabId).toBe(101)

    const [reElected] = sessionsToLiveCards(
      [
        liveSession({
          live: {
            state: 'active',
            browserTabs: [
              browserTab({ browserTabId: 102, lastActivityAt: 2_000 }),
              browserTab({ browserTabId: 103, lastActivityAt: 3_000 }),
            ],
          },
        }),
      ],
      { stickySelection: new Map([['session-1', 101]]) },
    )
    expect(reElected.selectedTab?.browserTabId).toBe(103)
  })

  it('uses parent session identity and existing fallbacks', () => {
    const [card] = sessionsToLiveCards([
      liveSession({
        profileId: 'profile-parent',
        slug: 'parent-slug',
        label: '',
        harness: undefined,
        color: undefined,
      }),
    ])

    expect(card).toMatchObject({
      profileId: 'profile-parent',
      label: 'parent-slug',
      harness: 'Claude Code',
      color: colorForSlug('parent-slug'),
    })
  })

  it('sorts cards by session start and session id, never activity recency', () => {
    const cards = sessionsToLiveCards([
      liveSession({ sessionId: 'session-c', startedAt: 200 }),
      liveSession({ sessionId: 'session-b', startedAt: 100 }),
      liveSession({ sessionId: 'session-a', startedAt: 100 }),
    ])

    expect(cards.map((card) => card.sessionId)).toEqual([
      'session-a',
      'session-b',
      'session-c',
    ])
  })
})
