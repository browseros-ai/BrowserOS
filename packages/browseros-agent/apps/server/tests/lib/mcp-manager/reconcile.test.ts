/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type {
  AddServerOptions,
  InstalledServer,
  LinkServerOptions,
  McpManager,
  RemoveServerOptions,
} from 'agent-mcp-manager'
import { createMcpManager } from 'agent-mcp-manager'
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

async function withTempMcpEnv<T>(
  run: (paths: {
    browserosDir: string
    claudeConfigPath: string
  }) => Promise<T>,
): Promise<T> {
  const root = await mkdtemp(join(tmpdir(), 'mcp-manager-reconcile-'))
  const previous = {
    BROWSEROS_DIR: process.env.BROWSEROS_DIR,
    CLAUDE_CONFIG_DIR: process.env.CLAUDE_CONFIG_DIR,
    HOME: process.env.HOME,
  }
  const browserosDir = join(root, 'browseros')
  const claudeConfigDir = join(root, 'claude-config')
  const homeDir = join(root, 'home')
  const claudeConfigPath = join(claudeConfigDir, '.claude.json')
  try {
    await mkdir(claudeConfigDir, { recursive: true })
    await mkdir(homeDir, { recursive: true })
    process.env.BROWSEROS_DIR = browserosDir
    process.env.CLAUDE_CONFIG_DIR = claudeConfigDir
    process.env.HOME = homeDir
    resetMcpManagerForTesting()
    return await run({ browserosDir, claudeConfigPath })
  } finally {
    resetMcpManagerForTesting()
    restoreEnv(previous)
    await rm(root, { recursive: true, force: true })
  }
}

function restoreEnv(previous: {
  BROWSEROS_DIR?: string
  CLAUDE_CONFIG_DIR?: string
  HOME?: string
}): void {
  for (const [key, value] of Object.entries(previous)) {
    if (value === undefined) {
      delete process.env[key]
    } else {
      process.env[key] = value
    }
  }
}

function makeManagerStub(initialServers: InstalledServer[]): {
  manager: McpManager
  calls: ManagerCalls
  setServers(servers: InstalledServer[]): void
  setLinkThrows(throws: Set<string>): void
  failNextAdd(err: Error): void
} {
  let servers = initialServers
  const calls: ManagerCalls = { add: [], link: [], remove: [] }
  let linkThrows = new Set<string>()
  let pendingAddFailure: Error | null = null

  const manager: McpManager = {
    add: mock(async (opts: AddServerOptions) => {
      calls.add.push(opts)
      if (pendingAddFailure) {
        const err = pendingAddFailure
        pendingAddFailure = null
        throw err
      }
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
    failNextAdd(err) {
      pendingAddFailure = err
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

  it('reapplies the claude-code http transport tag after URL drift', async () => {
    await withTempMcpEnv(async ({ browserosDir, claudeConfigPath }) => {
      await writeFile(claudeConfigPath, '{"mcpServers":{}}\n', 'utf8')
      const upstreamMgr = createMcpManager({
        workspaceDir: join(browserosDir, 'mcp-manager'),
        scope: 'system',
      })
      await upstreamMgr.add({
        name: 'browseros',
        spec: {
          transport: 'http',
          url: 'http://127.0.0.1:9100/mcp',
        },
      })
      await upstreamMgr.link({
        serverName: 'browseros',
        agent: 'claude-code',
      })

      expect(
        JSON.parse(await readFile(claudeConfigPath, 'utf8')).mcpServers
          .browseros,
      ).toEqual({
        url: 'http://127.0.0.1:9100/mcp',
      })

      resetMcpManagerForTesting()
      const result = await reconcileUrl({
        currentUrl: 'http://127.0.0.1:9105/mcp',
      })

      expect(result).toEqual({
        action: 'updated',
        affectedAgents: ['claude-code'],
      })
      expect(
        JSON.parse(await readFile(claudeConfigPath, 'utf8')).mcpServers
          .browseros,
      ).toEqual({
        url: 'http://127.0.0.1:9105/mcp',
        type: 'http',
      })
    })
  })

  it('best-effort restores the previous spec when add() throws after remove()', async () => {
    // Simulates the rare partial-write window: remove() succeeded but
    // add() failed (e.g. disk full while writing the manifest JSON).
    // Without rollback every linked agent would silently disconnect
    // with no way to recover until the next manual Connect click.
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
    stub.failNextAdd(new Error('disk full'))
    setMcpManagerForTesting(stub.manager)

    const result = await reconcileUrl({
      currentUrl: 'http://127.0.0.1:9105/mcp',
    })

    expect(result.action).toBe('updated')
    expect(result.affectedAgents).toEqual([])
    // First add() with the new url failed; rollback add() with the
    // original spec ran and succeeded.
    expect(stub.calls.add).toHaveLength(2)
    expect(stub.calls.add[1].spec).toEqual({
      transport: 'http',
      url: 'http://127.0.0.1:9100/mcp',
    })
    // No relink attempted after the failed rewrite — the entry exists
    // again but the previously-linked agents are not re-attached on
    // this pass.
    expect(stub.calls.link).toHaveLength(0)
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
