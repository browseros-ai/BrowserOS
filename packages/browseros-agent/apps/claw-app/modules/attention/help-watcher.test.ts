import { describe, expect, it } from 'bun:test'
import type { HelpRequest, SessionSummary } from '@browseros/claw-api'
import {
  attentionTitle,
  badgeText,
  diffHelp,
  openHelpRequests,
} from './attention.helpers'
import type { AttentionAlerts } from './attention-alerts'
import { type AnnouncedHelp, createHelpWatcher } from './help-watcher'

function session(
  sessionId: string,
  help?: Partial<HelpRequest>,
  label = 'Codex',
): SessionSummary {
  return {
    sessionId,
    slug: label.toLowerCase(),
    label,
    name: 'Task',
    startedAt: 1,
    durationMs: 1,
    dispatchCount: 1,
    toolSequence: [],
    status: 'live',
    errorCount: 0,
    live: help
      ? {
          state: 'active',
          browserTabs: [],
          helpRequest: {
            requestId: `req-${sessionId}`,
            reason: 'Enter the code',
            browserTabId: 7,
            url: 'https://www.netflix.com/login',
            requestedAt: 100,
            kind: 'login',
            ...help,
          } as HelpRequest,
        }
      : { state: 'active', browserTabs: [] },
  } as SessionSummary
}

function fakes() {
  const calls: string[] = []
  let stored: AnnouncedHelp[] = []
  const alerts: AttentionAlerts = {
    show: async (help) => {
      calls.push(`show ${help.requestId}`)
    },
    clear: async (requestId, tabId) => {
      calls.push(`clear ${requestId} tab ${tabId}`)
    },
    setCount: async (count) => {
      calls.push(`count ${count}`)
    },
  }
  const store = {
    load: async () => stored,
    save: async (next: AnnouncedHelp[]) => {
      stored = next
    },
  }
  return { calls, alerts, store, stored: () => stored }
}

describe('openHelpRequests', () => {
  it('says who needs what, without the agent’s own words', () => {
    const [help] = openHelpRequests([session('s1', {}), session('s2')])
    expect(help?.title).toBe('Codex needs you to sign in to netflix.com')
    expect(help?.message).not.toContain('Enter the code')
    expect(help?.browserTabId).toBe(7)
  })
})

describe('diffHelp', () => {
  it('finds new and finished requests', () => {
    const open = openHelpRequests([session('a', {}), session('b', {})])
    expect(diffHelp(['req-a', 'req-old'], open)).toEqual({
      started: [open[1] as (typeof open)[number]],
      ended: ['req-old'],
    })
  })
})

describe('badge and title', () => {
  it('stays quiet at zero and caps the count', () => {
    expect(badgeText(0)).toBe('')
    expect(badgeText(3)).toBe('3')
    expect(badgeText(12)).toBe('9+')
    expect(attentionTitle(0, 'neo')).toBe('neo')
    expect(attentionTitle(2, 'neo')).toBe('(2) Needs you · neo')
  })
})

describe('createHelpWatcher', () => {
  it('announces a request once, then clears it when it ends', async () => {
    const { calls, alerts, store } = fakes()
    const watcher = createHelpWatcher({
      alerts,
      store,
    })

    await watcher.sync([session('a', {})])
    await watcher.sync([session('a', {})])
    await watcher.sync([session('a')])

    expect(calls).toEqual([
      'show req-a',
      'count 1',
      'count 1',
      'clear req-a tab 7',
      'count 0',
    ])
  })

  it('does not announce twice when snapshots arrive together', async () => {
    const { calls, alerts, store } = fakes()
    const watcher = createHelpWatcher({
      alerts,
      store,
    })

    await Promise.all([
      watcher.sync([session('a', {})]),
      watcher.sync([session('a', {})]),
    ])

    expect(calls.filter((call) => call.startsWith('show'))).toEqual([
      'show req-a',
    ])
  })

  it('finds the request behind a notification for its buttons', async () => {
    const { alerts, store } = fakes()
    const watcher = createHelpWatcher({
      alerts,
      store,
    })
    await watcher.sync([session('a', {})])

    expect(await watcher.lookup('req-a')).toEqual({
      requestId: 'req-a',
      sessionId: 'a',
      browserTabId: 7,
    })
  })
})
