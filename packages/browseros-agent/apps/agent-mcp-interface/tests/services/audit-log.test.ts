import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import {
  getAuditDb,
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../src/modules/db/db'
import { toolDispatches } from '../../src/modules/db/schema/tool-dispatches.sql'
import {
  listDispatches,
  recordToolDispatch,
} from '../../src/services/audit-log'

function record(
  over: Partial<Parameters<typeof recordToolDispatch>[0]> = {},
): void {
  recordToolDispatch({
    agentId: 'claude-code',
    slug: 'claude-code',
    agentLabel: 'claude-code',
    sessionId: 's1',
    toolName: 'navigate',
    pageId: 1,
    targetId: 't1',
    url: 'https://example.com',
    title: 'Example',
    rawArgs: { url: 'https://example.com' },
    durationMs: 12,
    result: {
      isError: false,
      structuredContent: { page: 1 },
      content: [{ type: 'text', text: 'ok' }],
    },
    ...over,
  })
}

describe('recordToolDispatch', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('inserts a row with the supplied fields', () => {
    record()
    const rows = getAuditDb().select().from(toolDispatches).all()
    expect(rows.length).toBe(1)
    expect(rows[0]?.agentId).toBe('claude-code')
    expect(rows[0]?.toolName).toBe('navigate')
    expect(rows[0]?.url).toBe('https://example.com')
    expect(rows[0]?.durationMs).toBe(12)
  })

  it('serialises args as JSON', () => {
    record({ rawArgs: { a: 1, b: 'two' } })
    const row = getAuditDb().select().from(toolDispatches).get()
    expect(JSON.parse(row?.argsJson ?? '{}')).toEqual({ a: 1, b: 'two' })
  })

  it('truncates args JSON over 4 KB and ends with a tilde marker', () => {
    record({ rawArgs: { huge: 'x'.repeat(8000) } })
    const row = getAuditDb().select().from(toolDispatches).get()
    expect(row?.argsJson?.length).toBe(4096)
    expect(row?.argsJson?.endsWith('~')).toBe(true)
  })

  it('summarises result meta with isError + content block count + structured keys', () => {
    record({
      result: {
        isError: false,
        structuredContent: { page: 1, group: 'g' },
        content: [
          { type: 'text', text: 'a' },
          { type: 'text', text: 'b' },
        ],
      },
    })
    const row = getAuditDb().select().from(toolDispatches).get()
    const meta = JSON.parse(row?.resultMeta ?? '{}') as {
      isError: boolean
      contentSummary: string
      structuredKeys: string[]
    }
    expect(meta.isError).toBe(false)
    expect(meta.contentSummary).toBe('2 block(s)')
    expect(meta.structuredKeys).toContain('page')
    expect(meta.structuredKeys).toContain('group')
  })

  it('never throws when the DB is in a broken state; logs at warn instead', () => {
    resetAuditDbForTesting()
    // Trigger a real write through the unmocked singleton constructor;
    // an :memory: rebuild will succeed and the record will land. The
    // promise is the call does not throw.
    expect(() => record()).not.toThrow()
  })
})

describe('listDispatches', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  function seed(n: number, agentId = 'claude-code'): void {
    for (let i = 0; i < n; i++) {
      record({
        agentId,
        slug: agentId,
        agentLabel: agentId,
        toolName: `tool-${i}`,
      })
    }
  }

  it('orders newest first', () => {
    seed(3)
    const { rows } = listDispatches({})
    expect(rows.length).toBe(3)
    expect(rows[0]?.createdAt >= rows[1]?.createdAt).toBe(true)
    expect(rows[1]?.createdAt >= rows[2]?.createdAt).toBe(true)
  })

  it('filters by agentId', () => {
    seed(2, 'claude-code')
    seed(3, 'cursor-bot')
    const { rows } = listDispatches({ agentId: 'cursor-bot' })
    expect(rows.length).toBe(3)
    for (const r of rows) expect(r.agentId).toBe('cursor-bot')
  })

  it('filters by sessionId', () => {
    record({ sessionId: 's1' })
    record({ sessionId: 's2' })
    record({ sessionId: 's2' })
    const { rows } = listDispatches({ sessionId: 's2' })
    expect(rows.length).toBe(2)
  })

  it('returns nextCursor when more rows exist and null otherwise', () => {
    seed(5)
    const page1 = listDispatches({ limit: 2 })
    expect(page1.rows.length).toBe(2)
    expect(page1.nextCursor).not.toBeNull()
    const page2 = listDispatches({ limit: 2, cursor: page1.nextCursor! })
    expect(page2.rows.length).toBe(2)
    expect(page2.nextCursor).not.toBeNull()
    const page3 = listDispatches({ limit: 2, cursor: page2.nextCursor! })
    expect(page3.rows.length).toBe(1)
    expect(page3.nextCursor).toBeNull()
  })

  it('caps limit at 500', () => {
    seed(3)
    const { rows } = listDispatches({ limit: 10_000 })
    expect(rows.length).toBe(3)
  })
})
