import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import {
  getAuditDb,
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../src/modules/db/db'
import { sessionTabs } from '../../src/modules/db/schema/session-tabs.sql'
import {
  claimTabForSession,
  getOpenSessionTab,
  inheritTabOwnership,
  listOpenSessionTabs,
  releaseAllOpenSessionTabs,
  releaseTabForSession,
  releaseTabsForSession,
} from '../../src/services/session-tabs'

beforeEach(() => setAuditDbForTesting())
afterEach(() => resetAuditDbForTesting())

describe('session tab ownership', () => {
  it('closes the prior owner at the boundary before transferring a tab', () => {
    claimTabForSession({
      sessionId: 'session-a',
      agentId: 'agent-a',
      tabId: 11,
      openedTargetId: 'target-a',
      claimedAt: 100,
    })
    claimTabForSession({
      sessionId: 'session-b',
      agentId: 'agent-b',
      tabId: 11,
      openedTargetId: 'target-b',
      claimedAt: 200,
    })

    expect(getAuditDb().select().from(sessionTabs).all()).toEqual([
      expect.objectContaining({ sessionId: 'session-a', releasedAt: 200 }),
      expect.objectContaining({ sessionId: 'session-b', releasedAt: null }),
    ])
  })

  it('inherits the opener owner without changing the opener window', () => {
    claimTabForSession({
      sessionId: 'session-a',
      agentId: 'agent-a',
      tabId: 11,
      openedTargetId: 'target-a',
      claimedAt: 100,
    })
    inheritTabOwnership(11, 22, 'target-popup', 150)

    expect(getAuditDb().select().from(sessionTabs).all()).toEqual([
      expect.objectContaining({ tabId: 11, releasedAt: null }),
      expect.objectContaining({
        sessionId: 'session-a',
        agentId: 'agent-a',
        tabId: 22,
        openedTargetId: 'target-popup',
        claimedAt: 150,
        releasedAt: null,
      }),
    ])
  })

  it('releases only the requested live ownership windows', () => {
    for (const [sessionId, tabId] of [
      ['session-a', 11],
      ['session-a', 22],
      ['session-b', 33],
    ] as const) {
      claimTabForSession({
        sessionId,
        agentId: `agent-${sessionId}`,
        tabId,
        openedTargetId: null,
        claimedAt: 100,
      })
    }

    releaseTabForSession(11, 'session-a', 200)
    releaseTabsForSession('session-a', 300)
    releaseAllOpenSessionTabs(400)

    expect(
      getAuditDb()
        .select({
          tabId: sessionTabs.tabId,
          releasedAt: sessionTabs.releasedAt,
        })
        .from(sessionTabs)
        .all(),
    ).toEqual([
      { tabId: 11, releasedAt: 200 },
      { tabId: 22, releasedAt: 300 },
      { tabId: 33, releasedAt: 400 },
    ])
  })

  it('reads only current ownership and can resolve an exact session tab', () => {
    claimTabForSession({
      sessionId: 'session-a',
      agentId: 'agent-a',
      tabId: 11,
      openedTargetId: 'target-a',
      claimedAt: 100,
    })
    claimTabForSession({
      sessionId: 'session-b',
      agentId: 'agent-b',
      tabId: 22,
      openedTargetId: 'target-b',
      claimedAt: 110,
    })
    releaseTabForSession(11, 'session-a', 200)

    expect(listOpenSessionTabs()).toEqual([
      expect.objectContaining({ sessionId: 'session-b', tabId: 22 }),
    ])
    expect(getOpenSessionTab('session-b', 22)).toEqual(
      expect.objectContaining({ openedTargetId: 'target-b' }),
    )
    expect(getOpenSessionTab('session-a', 11)).toBeNull()
    expect(getOpenSessionTab('session-a', 22)).toBeNull()
  })
})
