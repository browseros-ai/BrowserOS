/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Unit tests for the replay-tabs deriver. Stubs the three deps
 * (registry, identity service, tab group tracker) so we can drive
 * the matrix of "agent has live session vs not", "tab group known
 * vs not", and same-name sessions without touching the singleton
 * state.
 */

import { describe, expect, it } from 'bun:test'
import { TAB_GROUP_COLORS } from '../../src/lib/agent-tab-groups/group-color'
import {
  agentIdentityFromClient,
  type ClientIdentity,
} from '../../src/lib/mcp-session'
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
        sessionLabel: null,
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

function identityFor(sessionId: string, clientName: string): ClientIdentity {
  return {
    sessionId,
    clientName,
    clientVersion: '0.0.1',
    clientTitle: null,
    sessionLabel: null,
    firstSeenAt: 1_000_000,
  }
}

function agentIdFor(sessionId: string, clientName: string): string {
  return agentIdentityFromClient(identityFor(sessionId, clientName)).agentId
}

describe('replay-tabs service', () => {
  it('emits one row per active tab, joined with sessionId + groupColor', () => {
    const agentId = agentIdFor('sid-abc', 'claude-code')
    const svc = createReplayTabsService({
      registry: registryStub([
        {
          agentId,
          pageId: 7,
          url: 'https://news.google.com/',
          title: 'Top stories',
        },
      ]),
      identityService: identityStub([
        { sessionId: 'sid-abc', clientName: 'claude-code' },
      ]),
      tabGroupTracker: tabGroupStub({
        [agentId]: { color: 'orange' },
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
    const agentId = agentIdFor('sid-1', 'claude-code')
    const svc = createReplayTabsService({
      registry: registryStub([
        {
          agentId,
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
    const agentId = agentIdFor('sid-1', 'a1')
    const svc = createReplayTabsService({
      registry: registryStub([
        { agentId, pageId: 1, url: 'https://a.com/', title: 'A' },
        { agentId, pageId: 2, url: 'https://b.com/', title: 'B' },
        { agentId, pageId: 3, url: 'https://c.com/', title: 'C' },
      ]),
      identityService: identityStub([{ sessionId: 'sid-1', clientName: 'a1' }]),
      tabGroupTracker: tabGroupStub({ [agentId]: { color: 'blue' } }),
    })
    const rows = svc.list()
    expect(rows).toHaveLength(3)
    for (const row of rows) {
      expect(row.sessionId).toBe('sid-1')
      expect(row.groupColor).toBe('blue')
    }
    expect(rows.map((r) => r.tabPageId).sort()).toEqual([1, 2, 3])
  })

  it('keeps same-name sessions distinct by session-scoped agentId', () => {
    const aAgentId = agentIdFor('sid-1', 'claude-code')
    const bAgentId = agentIdFor('sid-2', 'claude-code')
    const svc = createReplayTabsService({
      registry: registryStub([
        { agentId: aAgentId, pageId: 7, url: 'x', title: '' },
        { agentId: bAgentId, pageId: 8, url: 'y', title: '' },
      ]),
      identityService: identityStub([
        { sessionId: 'sid-1', clientName: 'claude-code' },
        { sessionId: 'sid-2', clientName: 'claude-code' },
      ]),
      tabGroupTracker: tabGroupStub({}),
    })
    const rows = svc.list()
    expect(rows).toHaveLength(2)
    const bySid = Object.fromEntries(rows.map((r) => [r.sessionId, r]))
    expect(bySid['sid-1'].tabPageId).toBe(7)
    expect(bySid['sid-2'].tabPageId).toBe(8)
  })

  it('distinct agents emit distinct sessions even when both are live', () => {
    const aAgentId = agentIdFor('sid-1', 'a1')
    const bAgentId = agentIdFor('sid-2', 'a2')
    const svc = createReplayTabsService({
      registry: registryStub([
        { agentId: aAgentId, pageId: 1, url: 'https://x.com/', title: 'x' },
        { agentId: bAgentId, pageId: 2, url: 'https://y.com/', title: 'y' },
      ]),
      identityService: identityStub([
        { sessionId: 'sid-1', clientName: 'a1' },
        { sessionId: 'sid-2', clientName: 'a2' },
      ]),
      tabGroupTracker: tabGroupStub({
        [aAgentId]: { color: 'orange' },
        [bAgentId]: { color: 'cyan' },
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
    const agentId = agentIdFor('sid-1', 'a1')
    const svc = createReplayTabsService({
      registry: registryStub([
        { agentId, pageId: 1, url: 'https://x.com/', title: 'x' },
      ]),
      identityService: identityStub([{ sessionId: 'sid-1', clientName: 'a1' }]),
      tabGroupTracker: tabGroupStub({ [agentId]: { color: 'orange' } }),
    })
    const rows = svc.list()
    expect(TAB_GROUP_COLORS).toContain(rows[0].groupColor as string)
  })
})
