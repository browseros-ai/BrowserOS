/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Fire-and-forget writers for agent_session_starts + agent_session_ends.
 * Both are pure inserts; SQLite hiccups log at warn and never throw,
 * matching the Phase 5 audit-log discipline.
 */

import { logger } from '../lib/logger'
import { getAuditDb } from '../modules/db/db'
import {
  agentSessionEnds,
  agentSessionStarts,
} from '../modules/db/schema/schema'

export interface RecordSessionStartInput {
  sessionId: string
  agentId: string
  slug: string
  agentLabel: string
  clientName: string
  clientVersion: string
}

/** Fire-and-forget. Never throws. */
export function recordSessionStart(input: RecordSessionStartInput): void {
  try {
    const db = getAuditDb()
    db.insert(agentSessionStarts).values(input).run()
  } catch (err) {
    logger.warn('session start write failed', {
      sessionId: input.sessionId,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}

export interface RecordSessionEndInput {
  sessionId: string
  kind: 'closed' | 'errored'
  reason?: string | null
}

/** Fire-and-forget. Never throws. */
export function recordSessionEnd(input: RecordSessionEndInput): void {
  try {
    const db = getAuditDb()
    db.insert(agentSessionEnds)
      .values({
        sessionId: input.sessionId,
        kind: input.kind,
        reason: input.reason ?? null,
      })
      .run()
  } catch (err) {
    logger.warn('session end write failed', {
      sessionId: input.sessionId,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}
