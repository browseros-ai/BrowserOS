/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { and, eq, isNull } from 'drizzle-orm'
import { logger } from '../lib/logger'
import { getAuditDb } from '../modules/db/db'
import { tabClaims } from '../modules/db/schema/tab-claims.sql'

export interface ClaimTargetInput {
  targetId: string
  sessionId: string
  agentId: string
  claimedAt: number
}

/** Records when an MCP session begins driving a browser target. */
export function claimTargetForSession(input: ClaimTargetInput): void {
  try {
    getAuditDb().insert(tabClaims).values(input).run()
  } catch (error) {
    logClaimWriteFailure('insert', { ...input }, error)
  }
}

/** Closes this session's open claim for a target after a successful tab close. */
export function releaseTargetForSession(
  targetId: string,
  sessionId: string,
): void {
  try {
    getAuditDb()
      .update(tabClaims)
      .set({ releasedAt: Date.now() })
      .where(
        and(
          eq(tabClaims.targetId, targetId),
          eq(tabClaims.sessionId, sessionId),
          isNull(tabClaims.releasedAt),
        ),
      )
      .run()
  } catch (error) {
    logClaimWriteFailure(
      'release-target-session',
      { targetId, sessionId },
      error,
    )
  }
}

/** Closes every open claim when an MCP session ends. */
export function releaseClaimsForSession(sessionId: string): void {
  try {
    getAuditDb()
      .update(tabClaims)
      .set({ releasedAt: Date.now() })
      .where(
        and(eq(tabClaims.sessionId, sessionId), isNull(tabClaims.releasedAt)),
      )
      .run()
  } catch (error) {
    logClaimWriteFailure('release-session', { sessionId }, error)
  }
}

/** Closes every open claim when CDP reports a target was destroyed. */
export function releaseClaimsForTarget(targetId: string): void {
  try {
    getAuditDb()
      .update(tabClaims)
      .set({ releasedAt: Date.now() })
      .where(
        and(eq(tabClaims.targetId, targetId), isNull(tabClaims.releasedAt)),
      )
      .run()
  } catch (error) {
    logClaimWriteFailure('release-target', { targetId }, error)
  }
}

/** Closes claims left open by sessions that can no longer be alive. */
export function releaseAllOpenClaims(releasedAt = Date.now()): void {
  try {
    getAuditDb()
      .update(tabClaims)
      .set({ releasedAt })
      .where(isNull(tabClaims.releasedAt))
      .run()
  } catch (error) {
    logClaimWriteFailure('release-all', {}, error)
  }
}

function logClaimWriteFailure(
  operation: string,
  fields: Record<string, unknown>,
  error: unknown,
): void {
  logger.warn('tab claim write failed', {
    operation,
    ...fields,
    error: error instanceof Error ? error.message : String(error),
  })
}
