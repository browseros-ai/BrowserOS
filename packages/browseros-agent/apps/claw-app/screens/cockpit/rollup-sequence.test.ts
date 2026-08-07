/**
 * Replays successive live-session snapshots and threads the selected
 * browser-tab map into the next projection. This pins the cockpit's
 * cross-poll selection semantics independently of React rendering.
 */

import { describe, expect, it } from 'bun:test'
import type { SessionBrowserTab, SessionSummary } from '@browseros/claw-api'
import { sessionsToLiveCards } from './cockpit.helpers'

function browserTab(
  browserTabId: number,
  lastActivityAt: number,
): SessionBrowserTab {
  return {
    browserTabId,
    url: `https://tab-${browserTabId}.example/`,
    title: `Tab ${browserTabId}`,
    firstActivityAt: lastActivityAt,
    lastActivityAt,
    lastToolName: 'snapshot',
    toolCount: 1,
    recentTools: [{ name: 'snapshot', at: lastActivityAt }],
  }
}

function session(browserTabs: SessionBrowserTab[]): SessionSummary {
  return {
    sessionId: 'session-1',
    slug: 'codex',
    label: 'Codex',
    name: 'Parallel browser work',
    startedAt: 0,
    durationMs: 0,
    dispatchCount: browserTabs.length,
    toolSequence: browserTabs.map(() => 'snapshot'),
    status: 'live',
    errorCount: 0,
    live: { state: 'active', browserTabs },
  }
}

describe('live-session selection across polls', () => {
  it('keeps the first selected tab through a burst, then re-elects on removal', () => {
    const polls = [
      [browserTab(7, 100)],
      [browserTab(7, 100), browserTab(8, 200)],
      [browserTab(7, 100), browserTab(8, 200), browserTab(9, 300)],
      [browserTab(8, 200), browserTab(9, 300)],
    ]
    let stickySelection = new Map<string, number>()
    const observed: number[] = []

    for (const browserTabs of polls) {
      const [card] = sessionsToLiveCards([session(browserTabs)], {
        stickySelection,
      })
      if (!card.selectedTab) throw new Error('expected a selected browser tab')
      observed.push(card.selectedTab.browserTabId)
      stickySelection = new Map([
        [card.sessionId, card.selectedTab.browserTabId],
      ])
    }

    expect(observed).toEqual([7, 7, 7, 9])
  })

  it('keeps independent selections for two sessions sharing one profile', () => {
    const sessionA = {
      ...session([browserTab(11, 100), browserTab(12, 200)]),
      sessionId: 'session-a',
      profileId: 'profile-shared',
    }
    const sessionB = {
      ...session([browserTab(21, 100), browserTab(22, 200)]),
      sessionId: 'session-b',
      profileId: 'profile-shared',
    }
    const cards = sessionsToLiveCards([sessionA, sessionB], {
      stickySelection: new Map([
        ['session-a', 11],
        ['session-b', 21],
      ]),
    })

    expect(
      Object.fromEntries(
        cards.map((card) => [card.sessionId, card.selectedTab?.browserTabId]),
      ),
    ).toEqual({ 'session-a': 11, 'session-b': 21 })
  })
})
