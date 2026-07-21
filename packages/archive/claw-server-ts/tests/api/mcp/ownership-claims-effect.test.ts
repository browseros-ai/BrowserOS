import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { and, eq, isNull } from 'drizzle-orm'
import { applyOwnershipClaims } from '../../../src/api/mcp/effects/ownership-claims'
import {
  getAuditDb,
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../../src/modules/db/db'
import { sessionTabs } from '../../../src/modules/db/schema/session-tabs.sql'

const ok = { isError: false, content: [], structuredContent: undefined }

beforeEach(() => setAuditDbForTesting())
afterEach(() => resetAuditDbForTesting())

describe('recording claims effect', () => {
  it('inserts a logical tab claim after tabs new succeeds', () => {
    applyOwnershipClaims({
      call: {
        sessionId: 'session-a',
        agent: { agentId: 'agent-a', slug: 'agent' },
        key: 'agent-a',
        session: {
          pages: {
            getInfo: () => ({ targetId: 'target-a', tabId: 101 }),
          },
        },
        flags: { newPage: true, closePage: false, listTabs: false },
      },
      result: { ...ok, structuredContent: { page: 7 } },
      startedAtMs: 123,
    } as never)

    const claim = getAuditDb().select().from(sessionTabs).get()
    expect(claim).toMatchObject({
      tabId: 101,
      openedTargetId: 'target-a',
      sessionId: 'session-a',
      agentId: 'agent-a',
      claimedAt: 123,
      releasedAt: null,
    })
  })

  it('releases the matching open claim after tabs close succeeds', () => {
    getAuditDb()
      .insert(sessionTabs)
      .values([
        {
          tabId: 102,
          openedTargetId: 'target-b',
          sessionId: 'session-b',
          agentId: 'agent-b',
          claimedAt: 100,
        },
        {
          tabId: 103,
          openedTargetId: 'target-c',
          sessionId: 'other-session',
          agentId: 'agent-c',
          claimedAt: 200,
        },
      ])
      .run()

    applyOwnershipClaims({
      call: {
        sessionId: 'session-b',
        agent: { agentId: 'agent-b', slug: 'agent' },
        key: 'agent-b',
        args: { page: 8 },
        pageSnapshot: {
          pageId: 8,
          tabId: 102,
          targetId: 'target-b',
          url: '',
          title: '',
        },
        flags: { newPage: false, closePage: true, listTabs: false },
      },
      result: ok,
    } as never)

    const released = getAuditDb()
      .select()
      .from(sessionTabs)
      .where(
        and(eq(sessionTabs.sessionId, 'session-b'), eq(sessionTabs.tabId, 102)),
      )
      .get()
    expect(released?.releasedAt).toBeNumber()
    expect(
      getAuditDb()
        .select()
        .from(sessionTabs)
        .where(
          and(
            eq(sessionTabs.sessionId, 'other-session'),
            isNull(sessionTabs.releasedAt),
          ),
        )
        .get(),
    ).toBeDefined()
  })
})
