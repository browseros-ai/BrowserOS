import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import type { McpServerLink } from 'agent-mcp-manager'
import { env } from '../../src/env'
import {
  resetMcpManagerForTesting,
  setMcpManagerForTesting,
} from '../../src/lib/mcp-manager'
import {
  connectBrowserosToHarness,
  disconnectBrowserosFromHarness,
  listBrowserosConnections,
} from '../../src/services/browseros-connect'
import { createStubMcpManager } from '../_helpers/stub-mcp-manager'

function stubWithLinks(links: McpServerLink[]) {
  const stub = createStubMcpManager()
  stub.listLinks = async () => links
  return stub
}

describe('connectBrowserosToHarness', () => {
  beforeEach(() => {
    env.proxyPort = null
    resetMcpManagerForTesting()
  })
  afterEach(() => {
    env.proxyPort = null
    resetMcpManagerForTesting()
  })

  it('writes a "browseros" entry with the canonical URL and links it to the right agent id', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const result = await connectBrowserosToHarness('Claude Code')
    expect(result.installed).toBe(true)
    expect(result.agentId).toBe('claude-code')
    const add = stub.calls.find((c) => c.method === 'add')
    expect(add).toBeDefined()
    const addPayload = add?.payload as {
      name: string
      spec: { transport: string; url?: string }
    }
    expect(addPayload.name).toBe('BrowserClaw')
    expect(addPayload.spec.transport).toBe('http')
    expect(addPayload.spec.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/mcp$/)
    expect(addPayload.spec.url).not.toContain('/cockpit')
    const link = stub.calls.find((c) => c.method === 'link')
    expect(link).toBeDefined()
    expect((link?.payload as { agent: string }).agent).toBe('claude-code')
    // The link call passes allowOverwrite: true so agent-mcp-manager
    // takes ownership of any prior on-disk BrowserClaw entry instead
    // of throwing ForeignEntryError. This is deliberate: BrowserClaw
    // is the app's own name and any prior entry under it belongs to
    // us in practice (relocated workspace, dev rebuild, prior manifest
    // version).
    expect((link?.payload as { allowOverwrite?: boolean }).allowOverwrite).toBe(
      true,
    )
  })

  it('writes a direct HTTP spec for Codex (http-capable since agent-mcp-manager 0.0.3)', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    await connectBrowserosToHarness('Codex')
    const add = stub.calls.find((c) => c.method === 'add')
    const payload = add?.payload as {
      spec: { transport: string; url?: string }
    }
    expect(payload.spec.transport).toBe('http')
    expect(payload.spec.url).toMatch(/^http:\/\/127\.0\.0\.1:\d+\/mcp$/)
    expect(payload.spec.url).not.toContain('/cockpit')
  })

  it('uses the trusted proxy port when server and proxy ports differ', async () => {
    env.proxyPort = 9512
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    await connectBrowserosToHarness('Claude Code')
    const add = stub.calls.find((c) => c.method === 'add')
    const payload = add?.payload as {
      spec: { transport: string; url?: string }
    }
    expect(payload.spec.url).toBe('http://127.0.0.1:9512/mcp')
  })

  it('relinks an existing managed entry so URL drift rewrites the harness config', async () => {
    env.proxyPort = 9512
    const stub = createStubMcpManager()
    stub.listLinks = async () => {
      stub.calls.push({
        method: 'listLinks',
        payload: { serverNames: ['BrowserClaw'] },
      })
      return [
        {
          serverName: 'BrowserClaw',
          agent: 'claude-code',
          configPath: '/tmp/stub-claude-code.json',
        },
      ]
    }
    setMcpManagerForTesting(stub)
    await connectBrowserosToHarness('Claude Code')
    expect(stub.calls.map((call) => call.method)).toEqual([
      'listLinks',
      'listServers',
      'add',
      'unlink',
      'link',
    ])
    const link = stub.calls.find((c) => c.method === 'link')
    expect((link?.payload as { agent: string }).agent).toBe('claude-code')
    expect(link?.payload).toMatchObject({
      configPath: '/tmp/stub-claude-code.json',
      allowOverwrite: true,
    })
  })

  it('restores the previous BrowserClaw link when replacement link fails', async () => {
    env.proxyPort = 9512
    const stub = createStubMcpManager()
    const previousSpec = {
      transport: 'http' as const,
      url: 'http://127.0.0.1:9200/mcp',
    }
    stub.listLinks = async () => {
      stub.calls.push({
        method: 'listLinks',
        payload: { serverNames: ['BrowserClaw'] },
      })
      return [
        {
          serverName: 'BrowserClaw',
          agent: 'claude-code',
          configPath: '/tmp/stub-claude-code.json',
        },
      ]
    }
    stub.listServers = async () => {
      stub.calls.push({ method: 'listServers', payload: {} })
      return [
        {
          name: 'BrowserClaw',
          spec: previousSpec,
          addedAt: '2026-07-02T00:00:00.000Z',
          links: {},
        },
      ]
    }
    let linkAttempts = 0
    stub.link = async (opts) => {
      stub.calls.push({ method: 'link', payload: opts })
      linkAttempts++
      if (linkAttempts === 1) throw new Error('write denied')
      return {
        serverName: opts.serverName,
        agent: opts.agent,
        configPath: opts.configPath ?? `/tmp/stub-${opts.agent}.json`,
        created: true,
      }
    }
    setMcpManagerForTesting(stub)

    const result = await connectBrowserosToHarness('Claude Code')

    expect(result.installed).toBe(false)
    expect(result.message).toContain('write denied')
    const addCalls = stub.calls.filter((c) => c.method === 'add')
    expect(addCalls).toHaveLength(2)
    expect(addCalls[0]?.payload).toMatchObject({
      name: 'BrowserClaw',
      spec: { url: 'http://127.0.0.1:9512/mcp' },
    })
    expect(addCalls[1]?.payload).toMatchObject({
      name: 'BrowserClaw',
      spec: previousSpec,
    })
    const linkCalls = stub.calls.filter((c) => c.method === 'link')
    expect(linkCalls).toHaveLength(2)
    expect(linkCalls[1]?.payload).toMatchObject({
      serverName: 'BrowserClaw',
      agent: 'claude-code',
      configPath: '/tmp/stub-claude-code.json',
      allowOverwrite: true,
    })
  })

  it('short-circuits as a no-op for BrowserOS-internal harnesses (Hermes, OpenClaw)', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const hermes = await connectBrowserosToHarness('Hermes')
    const openclaw = await connectBrowserosToHarness('OpenClaw')
    expect(hermes.installed).toBe(true)
    expect(hermes.agentId).toBeNull()
    expect(openclaw.installed).toBe(true)
    expect(openclaw.agentId).toBeNull()
    expect(stub.calls.find((c) => c.method === 'add')).toBeUndefined()
    expect(stub.calls.find((c) => c.method === 'link')).toBeUndefined()
  })
})

describe('disconnectBrowserosFromHarness', () => {
  beforeEach(() => resetMcpManagerForTesting())
  afterEach(() => resetMcpManagerForTesting())

  it('unlinks the browseros entry from the right agent', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const result = await disconnectBrowserosFromHarness('Cursor')
    expect(result.installed).toBe(false)
    expect(result.agentId).toBe('cursor')
    const unlink = stub.calls.find((c) => c.method === 'unlink')
    expect(unlink).toBeDefined()
    const unlinkPayload = unlink?.payload as {
      serverName: string
      agent: string
    }
    expect(unlinkPayload.serverName).toBe('BrowserClaw')
    expect(unlinkPayload.agent).toBe('cursor')
  })

  it('is a no-op for Hermes / OpenClaw', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const hermes = await disconnectBrowserosFromHarness('Hermes')
    expect(hermes.installed).toBe(false)
    expect(hermes.agentId).toBeNull()
    expect(stub.calls.find((c) => c.method === 'unlink')).toBeUndefined()
  })

  it('does NOT drop the shared BrowserClaw manifest entry when other agents remain linked', async () => {
    // Regression guard: previous version unconditionally called
    // mgr.remove() after unlink, which deleted the shared server
    // manifest entry and orphaned every other agent's on-disk link.
    // With the fix, remove is only called when listLinks reports
    // zero remaining links for BrowserClaw.
    const stub = createStubMcpManager()
    stub.listLinks = async () => [
      {
        serverName: 'BrowserClaw',
        agent: 'claude-code',
        configPath: '/tmp/stub-claude-code.json',
      },
    ]
    setMcpManagerForTesting(stub)
    const result = await disconnectBrowserosFromHarness('Cursor')
    expect(result.installed).toBe(false)
    expect(stub.calls.find((c) => c.method === 'unlink')).toBeDefined()
    expect(stub.calls.find((c) => c.method === 'remove')).toBeUndefined()
  })

  it('drops the shared BrowserClaw manifest entry when the last agent is disconnected', async () => {
    // Complement of the previous test: when listLinks returns
    // empty after the unlink, remove is called so the manifest
    // does not carry a stale zero-link entry.
    const stub = createStubMcpManager()
    // Default listLinks returns []; keeping default.
    setMcpManagerForTesting(stub)
    const result = await disconnectBrowserosFromHarness('Cursor')
    expect(result.installed).toBe(false)
    expect(stub.calls.find((c) => c.method === 'unlink')).toBeDefined()
    expect(stub.calls.find((c) => c.method === 'remove')).toBeDefined()
  })
})

describe('listBrowserosConnections', () => {
  beforeEach(() => resetMcpManagerForTesting())
  afterEach(() => resetMcpManagerForTesting())

  it('returns one row per harness; external harnesses report installed=false when listLinks is empty', async () => {
    setMcpManagerForTesting(stubWithLinks([]))
    const list = await listBrowserosConnections()
    expect(list.length).toBeGreaterThanOrEqual(9)
    const ccode = list.find((c) => c.harness === 'Claude Code')
    expect(ccode?.installed).toBe(false)
    const cursor = list.find((c) => c.harness === 'Cursor')
    expect(cursor?.installed).toBe(false)
  })

  it('marks Hermes and OpenClaw installed by definition (BrowserOS-internal)', async () => {
    setMcpManagerForTesting(stubWithLinks([]))
    const list = await listBrowserosConnections()
    expect(list.find((c) => c.harness === 'Hermes')?.installed).toBe(true)
    expect(list.find((c) => c.harness === 'OpenClaw')?.installed).toBe(true)
  })

  it('reports a harness as installed when listLinks returns a link for its agent id', async () => {
    setMcpManagerForTesting(
      stubWithLinks([
        {
          serverName: 'BrowserClaw',
          agent: 'claude-code',
          configPath: '/tmp/stub-claude-code.json',
        },
      ]),
    )
    const list = await listBrowserosConnections()
    expect(list.find((c) => c.harness === 'Claude Code')?.installed).toBe(true)
    expect(list.find((c) => c.harness === 'Claude Code')?.configPath).toBe(
      '/tmp/stub-claude-code.json',
    )
    expect(list.find((c) => c.harness === 'Cursor')?.installed).toBe(false)
  })

  it('skips broken links so a manifest entry whose disk row is gone reports not-installed', async () => {
    setMcpManagerForTesting(
      stubWithLinks([
        {
          serverName: 'BrowserClaw',
          agent: 'claude-code',
          configPath: '/tmp/stub-claude-code.json',
          broken: true,
        },
      ]),
    )
    const list = await listBrowserosConnections()
    expect(list.find((c) => c.harness === 'Claude Code')?.installed).toBe(false)
  })
})
