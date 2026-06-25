import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import {
  resetMcpManagerForTesting,
  setMcpManagerForTesting,
} from '../../../src/lib/mcp-manager'
import app from '../../../src/server'
import { createStubMcpManager } from '../../_helpers/stub-mcp-manager'

describe('/cockpit/connections route chain', () => {
  beforeEach(() => {
    resetMcpManagerForTesting()
    setMcpManagerForTesting(createStubMcpManager())
  })
  afterEach(() => resetMcpManagerForTesting())

  it('GET /connections lists one row per harness', async () => {
    const res = await app.fetch(
      new Request('http://localhost/connections', { method: 'GET' }),
    )
    expect(res.status).toBe(200)
    const body = (await res.json()) as {
      connections: Array<{ harness: string; installed: boolean }>
    }
    expect(body.connections.length).toBeGreaterThanOrEqual(9)
    expect(
      body.connections.find((c) => c.harness === 'Claude Code'),
    ).toBeDefined()
  })

  it('POST /connections/:harness/connect connects a single harness', async () => {
    const res = await app.fetch(
      new Request(
        `http://localhost/connections/${encodeURIComponent('Claude Code')}/connect`,
        { method: 'POST' },
      ),
    )
    expect(res.status).toBe(200)
    const body = (await res.json()) as {
      installed: boolean
      agentId: string | null
    }
    expect(body.installed).toBe(true)
    expect(body.agentId).toBe('claude-code')
  })

  it('POST /connections/:harness/disconnect disconnects a single harness', async () => {
    const res = await app.fetch(
      new Request(
        `http://localhost/connections/${encodeURIComponent('Cursor')}/disconnect`,
        { method: 'POST' },
      ),
    )
    expect(res.status).toBe(200)
    const body = (await res.json()) as {
      installed: boolean
      agentId: string | null
    }
    expect(body.installed).toBe(false)
    expect(body.agentId).toBe('cursor')
  })

  it('rejects an unknown harness with a 400 (zValidator)', async () => {
    const res = await app.fetch(
      new Request('http://localhost/connections/NotAHarness/connect', {
        method: 'POST',
      }),
    )
    expect(res.status).toBe(400)
  })
})
