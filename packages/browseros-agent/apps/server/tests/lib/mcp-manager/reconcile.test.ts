/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import type {
  AddServerOptions,
  InstalledServer,
  LinkServerOptions,
  McpManager,
  RemoveServerOptions,
} from 'agent-mcp-manager'
import {
  reconcileUrl,
  resetMcpManagerForTesting,
  setMcpManagerForTesting,
} from '../../../src/lib/mcp-manager'

interface ManagerCalls {
  add: AddServerOptions[]
  link: LinkServerOptions[]
  remove: RemoveServerOptions[]
}

function makeManagerStub(initialServers: InstalledServer[]): {
  manager: McpManager
  calls: ManagerCalls
  setServers(servers: InstalledServer[]): void
  setLinkThrows(throws: Set<string>): void
} {
  let servers = initialServers
  const calls: ManagerCalls = { add: [], link: [], remove: [] }
  let linkThrows = new Set<string>()

  const manager: McpManager = {
    add: mock(async (opts: AddServerOptions) => {
      calls.add.push(opts)
      return { name: opts.name, created: true }
    }),
    link: mock(async (opts: LinkServerOptions) => {
      calls.link.push(opts)
      if (linkThrows.has(opts.agent)) {
        throw new Error(`Permission denied for ${opts.agent}`)
      }
      return {
        serverName: opts.serverName,
        agent: opts.agent,
        configPath: `/tmp/fake/${opts.agent}.json`,
        created: true,
      }
    }),
    unlink: mock(async () => ({
      serverName: '',
      agent: 'claude-code' as const,
      configPath: '',
      removed: true,
    })),
    remove: mock(async (opts: RemoveServerOptions) => {
      calls.remove.push(opts)
      servers = servers.filter((s) => s.name !== opts.serverName)
    }),
    listServers: mock(async () => servers),
    listLinks: mock(async () => []),
    rescan: mock(async () => ({
      verified: [],
      drifted: [],
      broken: [],
      unmanaged: [],
    })),
  }

  return {
    manager,
    calls,
    setServers(next) {
      servers = next
    },
    setLinkThrows(next) {
      linkThrows = next
    },
  }
}

beforeEach(() => {
  resetMcpManagerForTesting()
})

afterEach(() => {
  resetMcpManagerForTesting()
})

describe('reconcileUrl', () => {
  it('returns noop when no browseros entry exists in the manifest', async () => {
    const stub = makeManagerStub([])
    setMcpManagerForTesting(stub.manager)

    const result = await reconcileUrl({
      currentUrl: 'http://127.0.0.1:9100/mcp',
    })

    expect(result).toEqual({ action: 'noop', affectedAgents: [] })
    expect(stub.calls.add).toHaveLength(0)
    expect(stub.calls.link).toHaveLength(0)
    expect(stub.calls.remove).toHaveLength(0)
  })

  it('returns noop when the manifest url already matches the running url', async () => {
    const stub = makeManagerStub([
      {
        name: 'browseros',
        spec: { transport: 'http', url: 'http://127.0.0.1:9100/mcp' },
        addedAt: '2026-06-11T00:00:00.000Z',
        links: {
          'claude-code': {
            configPath: '/tmp/fake/claude-code.json',
            createdAt: '2026-06-11T00:00:00.000Z',
          },
        },
      },
    ])
    setMcpManagerForTesting(stub.manager)

    const result = await reconcileUrl({
      currentUrl: 'http://127.0.0.1:9100/mcp',
    })

    expect(result).toEqual({ action: 'noop', affectedAgents: [] })
    expect(stub.calls.remove).toHaveLength(0)
    expect(stub.calls.add).toHaveLength(0)
    expect(stub.calls.link).toHaveLength(0)
  })

  it('replays remove + add + relink for every linked agent when the url drifted', async () => {
    const stub = makeManagerStub([
      {
        name: 'browseros',
        spec: { transport: 'http', url: 'http://127.0.0.1:9100/mcp' },
        addedAt: '2026-06-11T00:00:00.000Z',
        links: {
          'claude-code': {
            configPath: '/tmp/fake/claude-code.json',
            createdAt: '2026-06-11T00:00:00.000Z',
          },
          cursor: {
            configPath: '/tmp/fake/cursor.json',
            createdAt: '2026-06-11T00:00:00.000Z',
          },
        },
      },
    ])
    setMcpManagerForTesting(stub.manager)

    const result = await reconcileUrl({
      currentUrl: 'http://127.0.0.1:9105/mcp',
    })

    expect(result.action).toBe('updated')
    expect(result.affectedAgents.sort()).toEqual(['claude-code', 'cursor'])
    expect(stub.calls.remove).toHaveLength(1)
    expect(stub.calls.remove[0]).toEqual({
      serverName: 'browseros',
      unlinkFirst: true,
    })
    expect(stub.calls.add).toHaveLength(1)
    expect(stub.calls.add[0].spec).toEqual({
      transport: 'http',
      url: 'http://127.0.0.1:9105/mcp',
    })
    expect(stub.calls.link.map((l) => l.agent).sort()).toEqual([
      'claude-code',
      'cursor',
    ])
  })

  it('warn-logs a per-agent failure without aborting the rest of the reconcile', async () => {
    const stub = makeManagerStub([
      {
        name: 'browseros',
        spec: { transport: 'http', url: 'http://127.0.0.1:9100/mcp' },
        addedAt: '2026-06-11T00:00:00.000Z',
        links: {
          'claude-code': {
            configPath: '/tmp/fake/claude-code.json',
            createdAt: '2026-06-11T00:00:00.000Z',
          },
          cursor: {
            configPath: '/tmp/fake/cursor.json',
            createdAt: '2026-06-11T00:00:00.000Z',
          },
        },
      },
    ])
    stub.setLinkThrows(new Set(['cursor']))
    setMcpManagerForTesting(stub.manager)

    const result = await reconcileUrl({
      currentUrl: 'http://127.0.0.1:9105/mcp',
    })

    expect(result.action).toBe('updated')
    expect(result.affectedAgents).toEqual(['claude-code'])
  })
})
