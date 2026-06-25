/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { eq } from 'drizzle-orm'
import {
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../src/modules/db/db'
import {
  agentSessionEnds,
  agentSessionStarts,
} from '../../src/modules/db/schema/schema'
import {
  recordSessionEnd,
  recordSessionStart,
} from '../../src/services/session-events'

describe('session events', () => {
  beforeEach(() => {
    setAuditDbForTesting()
  })

  afterEach(() => {
    resetAuditDbForTesting()
  })

  it('recordSessionStart writes one queryable row', () => {
    const db = setAuditDbForTesting()
    recordSessionStart({
      sessionId: 'sid-2',
      agentId: 'cursor',
      slug: 'cursor',
      agentLabel: 'Cursor',
      clientName: 'cursor',
      clientVersion: '1.0.0',
    })
    const rows = db
      .select()
      .from(agentSessionStarts)
      .where(eq(agentSessionStarts.sessionId, 'sid-2'))
      .all()
    expect(rows).toHaveLength(1)
    expect(rows[0]!.agentId).toBe('cursor')
    expect(rows[0]!.clientVersion).toBe('1.0.0')
  })

  it('recordSessionEnd kind="closed" lands a row', () => {
    const db = setAuditDbForTesting()
    recordSessionEnd({ sessionId: 'sid-3', kind: 'closed' })
    const rows = db
      .select()
      .from(agentSessionEnds)
      .where(eq(agentSessionEnds.sessionId, 'sid-3'))
      .all()
    expect(rows).toHaveLength(1)
    expect(rows[0]!.kind).toBe('closed')
    expect(rows[0]!.reason).toBeNull()
  })

  it('recordSessionEnd kind="errored" carries the reason', () => {
    const db = setAuditDbForTesting()
    recordSessionEnd({
      sessionId: 'sid-4',
      kind: 'errored',
      reason: 'transport broke',
    })
    const rows = db
      .select()
      .from(agentSessionEnds)
      .where(eq(agentSessionEnds.sessionId, 'sid-4'))
      .all()
    expect(rows).toHaveLength(1)
    expect(rows[0]!.kind).toBe('errored')
    expect(rows[0]!.reason).toBe('transport broke')
  })
})
