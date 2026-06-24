import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import {
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../../src/modules/db/db'
import app from '../../../src/server'
import { recordToolDispatch } from '../../../src/services/audit-log'

function seed(over: { agentId?: string; sessionId?: string } = {}): void {
  recordToolDispatch({
    agentId: over.agentId ?? 'claude-code',
    slug: over.agentId ?? 'claude-code',
    agentLabel: over.agentId ?? 'claude-code',
    sessionId: over.sessionId ?? 's1',
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
  })
}

describe('GET /cockpit/audit/dispatches', () => {
  beforeEach(() => setAuditDbForTesting())
  afterEach(() => resetAuditDbForTesting())

  it('returns an empty rows array and null nextCursor against a fresh DB', async () => {
    const res = await app.fetch(
      new Request('http://localhost/audit/dispatches'),
    )
    expect(res.status).toBe(200)
    const body = (await res.json()) as {
      rows: unknown[]
      nextCursor: number | null
    }
    expect(body.rows).toEqual([])
    expect(body.nextCursor).toBeNull()
  })

  it('surfaces seeded rows newest-first', async () => {
    seed({ agentId: 'a' })
    seed({ agentId: 'b' })
    seed({ agentId: 'c' })
    const res = await app.fetch(
      new Request('http://localhost/audit/dispatches'),
    )
    const body = (await res.json()) as {
      rows: Array<{ agentId: string }>
    }
    expect(body.rows.length).toBe(3)
    expect(body.rows[0]?.agentId).toBe('c')
  })

  it('filters by agentId', async () => {
    seed({ agentId: 'a' })
    seed({ agentId: 'b' })
    seed({ agentId: 'a' })
    const res = await app.fetch(
      new Request('http://localhost/audit/dispatches?agentId=a'),
    )
    const body = (await res.json()) as { rows: Array<{ agentId: string }> }
    expect(body.rows.length).toBe(2)
    expect(body.rows.every((r) => r.agentId === 'a')).toBe(true)
  })

  it('paginates with cursor and surfaces nextCursor while more rows exist', async () => {
    for (let i = 0; i < 5; i++) seed({ agentId: 'a' })
    const r1 = await app.fetch(
      new Request('http://localhost/audit/dispatches?limit=2'),
    )
    const b1 = (await r1.json()) as {
      rows: Array<{ id: number }>
      nextCursor: number | null
    }
    expect(b1.rows.length).toBe(2)
    expect(b1.nextCursor).not.toBeNull()
    const r2 = await app.fetch(
      new Request(
        `http://localhost/audit/dispatches?limit=2&cursor=${b1.nextCursor}`,
      ),
    )
    const b2 = (await r2.json()) as {
      rows: Array<{ id: number }>
      nextCursor: number | null
    }
    expect(b2.rows.length).toBe(2)
    expect(b2.nextCursor).not.toBeNull()
    const r3 = await app.fetch(
      new Request(
        `http://localhost/audit/dispatches?limit=2&cursor=${b2.nextCursor}`,
      ),
    )
    const b3 = (await r3.json()) as {
      rows: Array<{ id: number }>
      nextCursor: number | null
    }
    expect(b3.rows.length).toBe(1)
    expect(b3.nextCursor).toBeNull()
  })

  it('rejects limit above the cap with a 400', async () => {
    const res = await app.fetch(
      new Request('http://localhost/audit/dispatches?limit=10000'),
    )
    expect(res.status).toBe(400)
  })
})
