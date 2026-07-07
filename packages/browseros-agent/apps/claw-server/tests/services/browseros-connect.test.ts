/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
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

const ORIGINAL_SERVER_PORT = env.serverPort

describe('connectBrowserosToHarness', () => {
  beforeEach(() => {
    env.proxyPort = null
    env.serverPort = ORIGINAL_SERVER_PORT
    resetMcpManagerForTesting()
  })
  afterEach(() => {
    env.proxyPort = null
    env.serverPort = ORIGINAL_SERVER_PORT
    resetMcpManagerForTesting()
  })

  it('writes a "BrowserClaw" entry with the canonical URL and links it to claude-code', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const result = await connectBrowserosToHarness('Claude Code')
    expect(result.installed).toBe(true)
    expect(result.agentId).toBe('claude-code')
    const link = stub.calls.find((c) => c.method === 'link')
    expect(link).toBeDefined()
    const payload = link?.payload as {
      server: { name: string; spec: { transport: string; url?: string } }
      agent: string
      allowOverwrite?: boolean
    }
    expect(payload.server.name).toBe('BrowserClaw')
    expect(payload.server.spec.transport).toBe('http')
    expect(payload.server.spec.url).toBe('http://127.0.0.1:9200/mcp')
    expect(payload.agent).toBe('claude-code')
    // BrowserClaw is the app's own name; any prior on-disk entry
    // under it belongs to us in practice (relocated workspace, dev
    // rebuild, prior manifest). allowOverwrite skips the library's
    // ForeignEntryError safety net for this specific server name.
    expect(payload.allowOverwrite).toBe(true)
  })

  it('writes a direct HTTP spec for Codex (http-capable in the catalog)', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const result = await connectBrowserosToHarness('Codex')
    expect(result.installed).toBe(true)
    const link = stub.calls.find((c) => c.method === 'link')
    const payload = link?.payload as {
      server: { spec: { transport: string; url?: string } }
    }
    expect(payload.server.spec.transport).toBe('http')
    expect(payload.server.spec.url).toBe('http://127.0.0.1:9200/mcp')
  })

  it('falls back to the server bind port when no proxy port is configured', async () => {
    env.proxyPort = null
    env.serverPort = 9500
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    await connectBrowserosToHarness('Cursor')
    const link = stub.calls.find((c) => c.method === 'link')
    expect(link).toBeDefined()
    const url = (link!.payload as { server: { spec: { url: string } } }).server
      .spec.url
    expect(url).toBe('http://127.0.0.1:9500/mcp')
  })

  it('uses the proxy port when server and proxy ports differ', async () => {
    env.proxyPort = 9300
    env.serverPort = 9500
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    await connectBrowserosToHarness('Cursor')
    const link = stub.calls.find((c) => c.method === 'link')
    expect(link).toBeDefined()
    const url = (link!.payload as { server: { spec: { url: string } } }).server
      .spec.url
    expect(url).toBe('http://127.0.0.1:9300/mcp')
  })

  it('relinks the existing managed entry when the URL drifts', async () => {
    const stub = createStubMcpManager()
    stub.seedServer({
      name: 'BrowserClaw',
      spec: { transport: 'http', url: 'http://127.0.0.1:8080/mcp' },
    })
    setMcpManagerForTesting(stub)
    env.proxyPort = 9250
    await connectBrowserosToHarness('Claude Code')
    const link = stub.calls.find((c) => c.method === 'link')
    expect(link).toBeDefined()
    const url = (link!.payload as { server: { spec: { url: string } } }).server
      .spec.url
    expect(url).toBe('http://127.0.0.1:9250/mcp')
  })

  it('restores the previous BrowserClaw spec when the replacement link throws', async () => {
    const stub = createStubMcpManager()
    stub.seedServer({
      name: 'BrowserClaw',
      spec: { transport: 'http', url: 'http://127.0.0.1:8080/mcp' },
    })
    // First link call throws; the second (restore) succeeds via the
    // default in-memory link behaviour.
    let calls = 0
    const originalLink = stub.link
    stub.link = async (input) => {
      calls++
      if (calls === 1) throw new Error('replacement failed')
      return originalLink(input)
    }
    setMcpManagerForTesting(stub)
    const result = await connectBrowserosToHarness('Claude Code')
    expect(result.installed).toBe(false)
    expect(result.message).toContain('replacement failed')
    // The manifest should still hold the previous spec after the
    // restore. Verify by listing servers.
    const servers = await stub.list()
    const bc = servers.find((s) => s.name === 'BrowserClaw')
    expect(bc?.spec).toMatchObject({
      transport: 'http',
      url: 'http://127.0.0.1:8080/mcp',
    })
  })
})

describe('disconnectBrowserosFromHarness', () => {
  beforeEach(() => resetMcpManagerForTesting())
  afterEach(() => resetMcpManagerForTesting())

  it('calls disconnect with removeIfLast:true for the requested agent', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const result = await disconnectBrowserosFromHarness('Cursor')
    expect(result.installed).toBe(false)
    expect(result.agentId).toBe('cursor')
    const disc = stub.calls.find((c) => c.method === 'disconnect')
    expect(disc?.payload).toMatchObject({
      serverName: 'BrowserClaw',
      agent: 'cursor',
      removeIfLast: true,
    })
  })

  it('leaves the shared manifest entry alive when other agents remain linked', async () => {
    const stub = createStubMcpManager()
    await stub.link({
      server: {
        name: 'BrowserClaw',
        spec: { transport: 'http', url: 'http://127.0.0.1:9200/mcp' },
      },
      agent: 'claude-code',
    })
    await stub.link({
      server: {
        name: 'BrowserClaw',
        spec: { transport: 'http', url: 'http://127.0.0.1:9200/mcp' },
      },
      agent: 'cursor',
    })
    setMcpManagerForTesting(stub)
    stub.reset()
    await disconnectBrowserosFromHarness('Cursor')
    const servers = await stub.list()
    const bc = servers.find((s) => s.name === 'BrowserClaw')
    expect(bc).toBeDefined()
    expect(bc?.links['claude-code']).toBeDefined()
    expect(bc?.links.cursor).toBeUndefined()
  })

  it('drops the shared manifest entry when the last agent is disconnected', async () => {
    const stub = createStubMcpManager()
    await stub.link({
      server: {
        name: 'BrowserClaw',
        spec: { transport: 'http', url: 'http://127.0.0.1:9200/mcp' },
      },
      agent: 'zed',
    })
    setMcpManagerForTesting(stub)
    stub.reset()
    await disconnectBrowserosFromHarness('Zed')
    const servers = await stub.list()
    expect(servers.find((s) => s.name === 'BrowserClaw')).toBeUndefined()
  })
})

describe('listBrowserosConnections', () => {
  beforeEach(() => resetMcpManagerForTesting())
  afterEach(() => resetMcpManagerForTesting())

  it('returns one row per supported harness, installed=false when no links', async () => {
    const stub = createStubMcpManager()
    setMcpManagerForTesting(stub)
    const rows = await listBrowserosConnections()
    expect(rows.length).toBe(7)
    for (const row of rows) expect(row.installed).toBe(false)
  })

  it('marks a harness installed when listLinks returns a link for its agent id', async () => {
    const stub = createStubMcpManager()
    await stub.link({
      server: {
        name: 'BrowserClaw',
        spec: { transport: 'http', url: 'http://127.0.0.1:9200/mcp' },
      },
      agent: 'cursor',
    })
    setMcpManagerForTesting(stub)
    const rows = await listBrowserosConnections()
    const cursorRow = rows.find((r) => r.harness === 'Cursor')
    expect(cursorRow?.installed).toBe(true)
    expect(cursorRow?.configPath).toBeDefined()
    const codexRow = rows.find((r) => r.harness === 'Codex')
    expect(codexRow?.installed).toBe(false)
  })

  it('hides a harness whose agent isInstalled probe returns false', async () => {
    const stub = createStubMcpManager()
    // Override the default (all-true) install probe: opencode + antigravity
    // report false; everything else true.
    stub.isInstalled = async (input) => {
      const out: Partial<Record<string, boolean>> = {}
      for (const agent of input.agents) {
        out[agent] = agent !== 'opencode' && agent !== 'antigravity'
      }
      return out as never
    }
    setMcpManagerForTesting(stub)
    const rows = await listBrowserosConnections()
    expect(rows.map((r) => r.harness)).toEqual([
      'Claude Code',
      'Codex',
      'Cursor',
      'VS Code',
      'Zed',
    ])
  })

  it('keeps an already-linked harness visible even if isInstalled reports false', async () => {
    const stub = createStubMcpManager()
    await stub.link({
      server: {
        name: 'BrowserClaw',
        spec: { transport: 'http', url: 'http://127.0.0.1:9200/mcp' },
      },
      agent: 'opencode',
    })
    // Even though the install probe says opencode is not installed,
    // the linked record means we already have a working install for
    // it. Keep the row so the user can disconnect.
    stub.isInstalled = async (input) => {
      const out: Partial<Record<string, boolean>> = {}
      for (const agent of input.agents) out[agent] = agent !== 'opencode'
      return out as never
    }
    setMcpManagerForTesting(stub)
    const rows = await listBrowserosConnections()
    const oc = rows.find((r) => r.harness === 'OpenCode')
    expect(oc?.installed).toBe(true)
  })

  it('collapses a home-relative configPath to ~ prefix in the row', async () => {
    const stub = createStubMcpManager()
    const HOME = (await import('node:os')).homedir()
    // Seed a linked entry whose configPath lives under the user's
    // home dir; the row should surface the tildified form so the
    // cockpit never renders the operator's username.
    await stub.link({
      server: {
        name: 'BrowserClaw',
        spec: { transport: 'http', url: 'http://127.0.0.1:9200/mcp' },
      },
      agent: 'zed',
      configPath: `${HOME}/.config/zed/settings.json`,
    })
    setMcpManagerForTesting(stub)
    const rows = await listBrowserosConnections()
    const zed = rows.find((r) => r.harness === 'Zed')
    expect(zed?.configPath).toBe('~/.config/zed/settings.json')
  })
})
