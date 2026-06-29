/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Unit tests for the replay-tabs deriver. Stubs the three deps
 * (registry, identity service, tab group tracker) so we can drive
 * the matrix of "agent has live session vs not", "tab group known
 * vs not", "multi-session-same-agentId" without touching the
 * singleton state.
 */

import { describe, expect, it } from 'bun:test'
import { TAB_GROUP_COLORS } from '../../src/lib/agent-tab-groups/group-color'
import { createReplayTabsService } from '../../src/services/replay-tabs'

function registryStub(records: Array<Record<string, unknown>>) {
  return {
    snapshot: () =>
      records as unknown as ReturnType<
        typeof import('../../src/lib/tab-activity').tabActivityRegistry.snapshot
      >,
  }
}

function identityStub(
  identities: Array<{
    sessionId: string
    clientName: string
    clientVersion?: string
    clientTitle?: string | null
    firstSeenAt?: number
  }>,
) {
  return {
    list: () =>
      identities.map((i) => ({
        sessionId: i.sessionId,
        clientName: i.clientName,
        clientVersion: i.clientVersion ?? '0.0.1',
        clientTitle: i.clientTitle ?? null,
        firstSeenAt: i.firstSeenAt ?? 1_000_000,
      })),
  }
}

function tabGroupStub(groups: Record<string, { color: string }>) {
  return {
    getByAgentId: (agentId: string) => {
      const g = groups[agentId]
      if (!g) return null
      // The real tracker returns the full record; we only need .color
      // for this consumer.
      return { color: g.color } as unknown as ReturnType<
        typeof import('../../src/lib/agent-tab-groups').tabGroupTracker.getByAgentId
      >
    },
  }
}

describe('replay-tabs service', () => {
  it('emits one row per active tab, joined with sessionId + groupColor', () => {
    const svc = createReplayTabsService({
      registry: registryStub([
        {
          agentId: 'claude-code',
          pageId: 7,
          url: 'https://news.google.com/',
          title: 'Top stories',
        },
      ]),
      identityService: identityStub([
        { sessionId: 'sid-abc', clientName: 'claude-code' },
      ]),
      tabGroupTracker: tabGroupStub({
        'claude-code': { color: 'orange' },
      }),
    })

    const rows = svc.list()
    expect(rows).toHaveLength(1)
    expect(rows[0]).toEqual({
      sessionId: 'sid-abc',
      tabPageId: 7,
      url: 'https://news.google.com/',
      title: 'Top stories',
      groupColor: 'orange',
    })
  })

  it('drops tabs whose agentId has no live identity (session ended)', () => {
    const svc = createReplayTabsService({
      registry: registryStub([
        { agentId: 'ghost-agent', pageId: 1, url: 'x', title: 'x' },
      ]),
      identityService: identityStub([]),
      tabGroupTracker: tabGroupStub({}),
    })
    expect(svc.list()).toEqual([])
  })

  it('emits groupColor:null when no tab group is registered yet', () => {
    const svc = createReplayTabsService({
      registry: registryStub([
        {
          agentId: 'claude-code',
          pageId: 7,
          url: 'https://news.google.com/',
          title: 'Top stories',
        },
      ]),
      identityService: identityStub([
        { sessionId: 'sid-1', clientName: 'claude-code' },
      ]),
      tabGroupTracker: tabGroupStub({}),
    })
    expect(svc.list()[0].groupColor).toBeNull()
  })

  it('handles multiple tabs for the same agent', () => {
    const svc = createReplayTabsService({
      registry: registryStub([
        { agentId: 'a1', pageId: 1, url: 'https://a.com/', title: 'A' },
        { agentId: 'a1', pageId: 2, url: 'https://b.com/', title: 'B' },
        { agentId: 'a1', pageId: 3, url: 'https://c.com/', title: 'C' },
      ]),
      identityService: identityStub([{ sessionId: 'sid-1', clientName: 'a1' }]),
      tabGroupTracker: tabGroupStub({ a1: { color: 'blue' } }),
    })
    const rows = svc.list()
    expect(rows).toHaveLength(3)
    for (const row of rows) {
      expect(row.sessionId).toBe('sid-1')
      expect(row.groupColor).toBe('blue')
    }
    expect(rows.map((r) => r.tabPageId).sort()).toEqual([1, 2, 3])
  })

  it('multi-session-same-agentId: picks one session per agent, documented limitation', () => {
    const svc = createReplayTabsService({
      registry: registryStub([
        { agentId: 'claude-code', pageId: 7, url: 'x', title: '' },
      ]),
      identityService: identityStub([
        { sessionId: 'sid-1', clientName: 'claude-code' },
        { sessionId: 'sid-2', clientName: 'claude-code' },
      ]),
      tabGroupTracker: tabGroupStub({}),
    })
    const rows = svc.list()
    expect(rows).toHaveLength(1)
    // We do not promise which session wins; only that exactly one does.
    expect(['sid-1', 'sid-2']).toContain(rows[0].sessionId)
  })

  it('distinct agents emit distinct sessions even when both are live', () => {
    const svc = createReplayTabsService({
      registry: registryStub([
        { agentId: 'a1', pageId: 1, url: 'https://x.com/', title: 'x' },
        { agentId: 'a2', pageId: 2, url: 'https://y.com/', title: 'y' },
      ]),
      identityService: identityStub([
        { sessionId: 'sid-1', clientName: 'a1' },
        { sessionId: 'sid-2', clientName: 'a2' },
      ]),
      tabGroupTracker: tabGroupStub({
        a1: { color: 'orange' },
        a2: { color: 'cyan' },
      }),
    })
    const rows = svc.list()
    expect(rows).toHaveLength(2)
    const bySid = Object.fromEntries(rows.map((r) => [r.sessionId, r]))
    expect(bySid['sid-1'].groupColor).toBe('orange')
    expect(bySid['sid-2'].groupColor).toBe('cyan')
  })

  it('emits groupColor strings that match the TabGroupColor enum', () => {
    // Defensive: the wire shape must only contain values from the
    // canonical TAB_GROUP_COLORS list so the extension's
    // chrome.tabGroups disambiguator stays valid.
    const svc = createReplayTabsService({
      registry: registryStub([
        { agentId: 'a1', pageId: 1, url: 'https://x.com/', title: 'x' },
      ]),
      identityService: identityStub([{ sessionId: 'sid-1', clientName: 'a1' }]),
      tabGroupTracker: tabGroupStub({ a1: { color: 'orange' } }),
    })
    const rows = svc.list()
    expect(TAB_GROUP_COLORS).toContain(rows[0].groupColor as string)
  })
})
