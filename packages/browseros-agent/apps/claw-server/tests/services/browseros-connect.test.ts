import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import type { McpServerLink } from 'agent-mcp-manager'
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
  beforeEach(() => resetMcpManagerForTesting())
  afterEach(() => resetMcpManagerForTesting())

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
    expect(addPayload.name).toBe('browseros')
    expect(addPayload.spec.transport).toBe('http')
    expect(addPayload.spec.url).toContain('/mcp')
    expect(addPayload.spec.url).not.toContain('/cockpit')
    const link = stub.calls.find((c) => c.method === 'link')
    expect(link).toBeDefined()
    expect((link?.payload as { agent: string }).agent).toBe('claude-code')
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
    expect(payload.spec.url).toContain('/mcp')
    expect(payload.spec.url).not.toContain('/cockpit')
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

  it('unlinks the browseros entry from the right agent and drops it from the manifest', async () => {
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
    expect(unlinkPayload.serverName).toBe('browseros')
    expect(unlinkPayload.agent).toBe('cursor')
    expect(stub.calls.find((c) => c.method === 'remove')).toBeDefined()
  })

  it('is a no-op for Hermes / OpenClaw', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const hermes = await disconnectBrowserosFromHarness('Hermes')
    expect(hermes.installed).toBe(false)
    expect(hermes.agentId).toBeNull()
    expect(stub.calls.find((c) => c.method === 'unlink')).toBeUndefined()
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
          serverName: 'browseros',
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
          serverName: 'browseros',
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
