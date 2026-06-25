import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { sql } from 'drizzle-orm'
import {
  getAuditDb,
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../../src/modules/db/db'
import { toolDispatches } from '../../../src/modules/db/schema/tool-dispatches.sql'

describe('audit DB (in-memory test seam)', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('runs migrations on construction so the tool_dispatches table is queryable', () => {
    const db = getAuditDb()
    const rows = db.select().from(toolDispatches).all()
    expect(rows).toEqual([])
  })

  it('records the unixepoch-derived createdAt default within a few seconds of now', () => {
    const db = getAuditDb()
    db.insert(toolDispatches)
      .values({
        agentId: 'a',
        slug: 'a',
        agentLabel: 'a',
        sessionId: 's',
        toolName: 'tabs',
      })
      .run()
    const row = db.select().from(toolDispatches).get()
    expect(row?.createdAt).toBeGreaterThan(Date.now() - 5_000)
    expect(row?.createdAt).toBeLessThanOrEqual(Date.now() + 1_000)
  })

  it('honours the three indexes defined in the schema', () => {
    const db = getAuditDb()
    const indexes = db
      .all<{ name: string }>(
        sql`SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='tool_dispatches'`,
      )
      .map((r) => r.name)
    expect(indexes).toContain('tool_dispatches_created_at_idx')
    expect(indexes).toContain('tool_dispatches_agent_created_idx')
    expect(indexes).toContain('tool_dispatches_session_idx')
  })

  it('reset drops the singleton; the next getAuditDb rebuilds a fresh DB', () => {
    const a = setAuditDbForTesting()
    a.insert(toolDispatches)
      .values({
        agentId: 'a',
        slug: 'a',
        agentLabel: 'a',
        sessionId: 's',
        toolName: 'navigate',
      })
      .run()
    expect(a.select().from(toolDispatches).all().length).toBe(1)
    resetAuditDbForTesting()
    const b = setAuditDbForTesting()
    expect(b.select().from(toolDispatches).all()).toEqual([])
  })
})
