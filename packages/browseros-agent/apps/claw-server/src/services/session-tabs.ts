/** Server-owned logical-tab ownership windows used by replay attribution. */

import { and, eq, isNull, type SQL } from 'drizzle-orm'
import { logger } from '../lib/logger'
import { getAuditDb } from '../modules/db/db'
import {
  type SessionTabRow,
  sessionTabs,
} from '../modules/db/schema/session-tabs.sql'

export interface ClaimTabInput {
  tabId: number
  openedTargetId: string | null
  sessionId: string
  agentId: string
  claimedAt: number
}

/** Returns the durable ownership windows that are still open. */
export function listOpenSessionTabs(): SessionTabRow[] {
  return getAuditDb()
    .select()
    .from(sessionTabs)
    .where(isNull(sessionTabs.releasedAt))
    .orderBy(sessionTabs.id)
    .all()
}

/** Resolves current ownership without exposing whether another session owns the tab. */
export function getOpenSessionTab(
  sessionId: string,
  tabId: number,
): SessionTabRow | null {
  return (
    getAuditDb()
      .select()
      .from(sessionTabs)
      .where(
        and(
          eq(sessionTabs.sessionId, sessionId),
          eq(sessionTabs.tabId, tabId),
          isNull(sessionTabs.releasedAt),
        ),
      )
      .get() ?? null
  )
}

/** Claims a logical tab, closing any stale live owner at the same boundary. */
export function claimTabForSession(input: ClaimTabInput): void {
  try {
    getAuditDb().transaction((tx) => {
      const existing = tx
        .select()
        .from(sessionTabs)
        .where(
          and(
            eq(sessionTabs.tabId, input.tabId),
            isNull(sessionTabs.releasedAt),
          ),
        )
        .get()
      if (
        existing?.sessionId === input.sessionId &&
        existing.agentId === input.agentId
      ) {
        return
      }
      if (existing) {
        tx.update(sessionTabs)
          .set({ releasedAt: input.claimedAt })
          .where(eq(sessionTabs.id, existing.id))
          .run()
      }
      tx.insert(sessionTabs).values(input).run()
    })
  } catch (error) {
    logWriteFailure('claim', { ...input }, error)
  }
}

/** Inherits the opener's live owner for a newly-created popup tab. */
export function inheritTabOwnership(
  openerTabId: number,
  tabId: number,
  openedTargetId: string,
  claimedAt = Date.now() - 1_000,
): void {
  try {
    const owner = getAuditDb()
      .select()
      .from(sessionTabs)
      .where(
        and(eq(sessionTabs.tabId, openerTabId), isNull(sessionTabs.releasedAt)),
      )
      .get()
    if (!owner) return
    claimTabForSession({
      tabId,
      openedTargetId,
      sessionId: owner.sessionId,
      agentId: owner.agentId,
      claimedAt,
    })
  } catch (error) {
    logWriteFailure(
      'inherit',
      { openerTabId, tabId, openedTargetId, claimedAt },
      error,
    )
  }
}

/** Closes this session's live window after a successful tab close. */
export function releaseTabForSession(
  tabId: number,
  sessionId: string,
  releasedAt = Date.now(),
): void {
  updateReleasedAt(
    'release-tab-session',
    and(
      eq(sessionTabs.tabId, tabId),
      eq(sessionTabs.sessionId, sessionId),
      isNull(sessionTabs.releasedAt),
    ),
    releasedAt,
    { tabId, sessionId },
  )
}

/** Closes every live tab when an MCP session ends. */
export function releaseTabsForSession(
  sessionId: string,
  releasedAt = Date.now(),
): void {
  updateReleasedAt(
    'release-session',
    and(eq(sessionTabs.sessionId, sessionId), isNull(sessionTabs.releasedAt)),
    releasedAt,
    { sessionId },
  )
}

/** Closes rows left live by sessions from an earlier server process. */
export function releaseAllOpenSessionTabs(releasedAt = Date.now()): void {
  updateReleasedAt(
    'release-all',
    isNull(sessionTabs.releasedAt),
    releasedAt,
    {},
  )
}

function updateReleasedAt(
  operation: string,
  where: SQL | undefined,
  releasedAt: number,
  fields: Record<string, unknown>,
): void {
  try {
    getAuditDb().update(sessionTabs).set({ releasedAt }).where(where).run()
  } catch (error) {
    logWriteFailure(operation, fields, error)
  }
}

function logWriteFailure(
  operation: string,
  fields: Record<string, unknown>,
  error: unknown,
): void {
  logger.warn('session tab write failed', {
    operation,
    ...fields,
    error: error instanceof Error ? error.message : String(error),
  })
}
