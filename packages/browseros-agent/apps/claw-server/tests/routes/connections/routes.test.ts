import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { env } from '../../../src/env'
import {
  resetMcpManagerForTesting,
  setMcpManagerForTesting,
} from '../../../src/lib/mcp-manager'
import app from '../../../src/server'
import { createStubMcpManager } from '../../_helpers/stub-mcp-manager'

describe('/connections route chain', () => {
  beforeEach(() => {
    env.proxyPort = null
    resetMcpManagerForTesting()
    setMcpManagerForTesting(createStubMcpManager())
  })
  afterEach(() => {
    env.proxyPort = null
    resetMcpManagerForTesting()
  })

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
    env.proxyPort = 9512
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
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
    const add = stub.calls.find((c) => c.method === 'add')
    expect((add?.payload as { spec: { url?: string } }).spec.url).toBe(
      'http://127.0.0.1:9512/mcp',
    )
  })

  it('ignores a caller-supplied MCP URL body', async () => {
    env.proxyPort = 9512
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const res = await app.fetch(
      new Request(
        `http://localhost/connections/${encodeURIComponent('Claude Code')}/connect`,
        {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ mcpUrl: 'http://127.0.0.1:7777/mcp' }),
        },
      ),
    )
    expect(res.status).toBe(200)
    const add = stub.calls.find((c) => c.method === 'add')
    expect((add?.payload as { spec: { url?: string } }).spec.url).toBe(
      'http://127.0.0.1:9512/mcp',
    )
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
