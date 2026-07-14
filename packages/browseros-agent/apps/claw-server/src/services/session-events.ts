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
import { bucketClientName, captureEvent } from './analytics'

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
  // Anonymous usage signal: a session happened and which agent ran it.
  // `client_name` is bucketed to a known set; no session/agent id, no
  // label, no content.
  captureEvent('agent_session_started', {
    client_name: bucketClientName(input.clientName),
  })
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
  // Session ended; `kind` distinguishes a clean close from an error.
  captureEvent('agent_session_ended', { kind: input.kind })
}
