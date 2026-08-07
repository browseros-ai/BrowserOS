import { describe, expect, test } from 'bun:test'

import {
  AgentNotInstalledError,
  ForeignEntryError,
  InvalidServerSpecError,
  UnsupportedTransportError,
} from '../../src/errors'
import {
  planDisconnect,
  planLink,
  planRemove,
  planRescan,
  planUnlink,
} from '../../src/planner/planner'
import type { State } from '../../src/planner/types'
import type {
  AgentId,
  ManifestServerEntry,
  McpServer,
  McpServerSpec,
  ServerManifest,
} from '../../src/types'

const NOW = '2026-07-06T12:00:00Z'

const STDIO_SPEC: McpServerSpec = {
  transport: 'stdio',
  command: 'gh-mcp',
  args: ['serve'],
}

const HTTP_SPEC: McpServerSpec = {
  transport: 'http',
  url: 'https://example.com/mcp',
}

const GH: McpServer = { name: 'gh', spec: STDIO_SPEC }
const GH_HTTP: McpServer = { name: 'gh', spec: HTTP_SPEC }

function emptyManifest(): ServerManifest {
  return { version: 1, servers: {} }
}

function serverEntry(
  overrides: Partial<ManifestServerEntry> = {},
): ManifestServerEntry {
  return {
    name: 'gh',
    spec: STDIO_SPEC,
    addedAt: NOW,
    links: {},
    ...overrides,
  }
}

function baseState(overrides: Partial<State> = {}): State {
  return {
    workspaceDir: '/tmp/ws',
    manifestPath: '/tmp/ws/manifest.json',
    manifest: emptyManifest(),
    agents: [],
    ...overrides,
  }
}

function stateWithServer(
  server: ManifestServerEntry,
  extras: Partial<State> = {},
): State {
  return baseState({
    manifest: { version: 1, servers: { [server.name]: server } },
    ...extras,
  })
}

function agentFile(
  agent: AgentId,
  configPath: string,
  raw = '',
  scope: 'system' | 'project' = 'system',
  parentExists = true,
) {
  return {
    agent,
    scope,
    configPath,
    rawContent: raw,
    exists: raw.length > 0,
    parentExists,
  }
}

// -------------------------------------------------------------------
// planLink — the primary write verb. Upserts manifest AND writes config.
// -------------------------------------------------------------------

describe('planLink', () => {
  test('first link creates a manifest server entry and writes agent config', () => {
    const state = baseState({
      agents: [agentFile('cursor', '/tmp/ws/cursor.json')],
    })
    const plan = planLink(state, { server: GH, agent: 'cursor' }, NOW)
    expect(plan.created).toBe(true)
    expect(plan.overwroteForeign).toBe(false)
    expect(plan.nextManifest.servers.gh?.spec).toEqual(STDIO_SPEC)
    expect(plan.nextManifest.servers.gh?.links.cursor?.configPath).toBe(
      '/tmp/ws/cursor.json',
    )
    expect(plan.ops).toHaveLength(2)
  })

  test('rejects an empty server name', () => {
    const state = baseState({
      agents: [agentFile('cursor', '/tmp/ws/cursor.json')],
    })
    expect(() =>
      planLink(
        state,
        { server: { name: '  ', spec: STDIO_SPEC }, agent: 'cursor' },
        NOW,
      ),
    ).toThrow(InvalidServerSpecError)
  })

  test('rejects a stdio spec with an empty command', () => {
    const state = baseState({
      agents: [agentFile('cursor', '/tmp/ws/cursor.json')],
    })
    expect(() =>
      planLink(
        state,
        {
          server: { name: 'x', spec: { transport: 'stdio', command: '' } },
          agent: 'cursor',
        },
        NOW,
      ),
    ).toThrow(InvalidServerSpecError)
  })

  test('trims the server name before using it as the manifest key', () => {
    const state = baseState({
      agents: [agentFile('cursor', '/tmp/ws/cursor.json')],
    })
    const plan = planLink(
      state,
      { server: { name: '  gh  ', spec: STDIO_SPEC }, agent: 'cursor' },
      NOW,
    )
    expect(plan.serverName).toBe('gh')
    expect(plan.nextManifest.servers.gh).toBeDefined()
    expect(plan.nextManifest.servers['  gh  ']).toBeUndefined()
  })

  test('re-linking the same server + agent returns created: false and preserves addedAt', () => {
    const state = stateWithServer(
      serverEntry({
        addedAt: '2020-01-01T00:00:00Z',
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
        },
      }),
      {
        agents: [
          agentFile(
            'cursor',
            '/tmp/ws/cursor.json',
            JSON.stringify({ mcpServers: { gh: { command: 'gh-mcp' } } }),
          ),
        ],
      },
    )
    const plan = planLink(state, { server: GH, agent: 'cursor' }, NOW)
    expect(plan.created).toBe(false)
    expect(plan.nextManifest.servers.gh?.addedAt).toBe('2020-01-01T00:00:00Z')
  })

  test('upserts manifest spec when the same name is linked with a different spec', () => {
    const state = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
        },
      }),
      {
        agents: [agentFile('gemini', '/tmp/ws/gemini.json', '')],
      },
    )
    const plan = planLink(state, { server: GH_HTTP, agent: 'gemini' }, NOW)
    // Manifest reflects the newer spec.
    expect(plan.nextManifest.servers.gh?.spec).toEqual(HTTP_SPEC)
    // Existing cursor link is preserved.
    expect(plan.nextManifest.servers.gh?.links.cursor).toBeDefined()
    // New gemini link is added.
    expect(plan.nextManifest.servers.gh?.links.gemini).toBeDefined()
  })

  test('throws UnsupportedTransportError when transport not accepted', () => {
    const state = baseState({
      agents: [agentFile('claude-desktop', '/tmp/ws/claude.json')],
    })
    expect(() =>
      planLink(state, { server: GH_HTTP, agent: 'claude-desktop' }, NOW),
    ).toThrow(UnsupportedTransportError)
  })

  test('throws ForeignEntryError when config already has an unmanaged entry under this name', () => {
    const foreignJson = JSON.stringify({
      mcpServers: { gh: { command: 'other-thing' } },
    })
    const state = baseState({
      agents: [agentFile('cursor', '/tmp/ws/cursor.json', foreignJson)],
    })
    expect(() => planLink(state, { server: GH, agent: 'cursor' }, NOW)).toThrow(
      ForeignEntryError,
    )
  })

  test('allowOverwrite: true takes ownership of a foreign entry', () => {
    const foreignJson = JSON.stringify({
      mcpServers: { gh: { command: 'other-thing' } },
    })
    const state = baseState({
      agents: [agentFile('cursor', '/tmp/ws/cursor.json', foreignJson)],
    })
    const plan = planLink(
      state,
      { server: GH, agent: 'cursor', allowOverwrite: true },
      NOW,
    )
    expect(plan.overwroteForeign).toBe(true)
    expect(plan.created).toBe(true)
  })

  test('re-linking with identical content skips the agent-config write op', () => {
    const state = baseState({
      agents: [agentFile('cursor', '/tmp/ws/cursor.json')],
    })
    const first = planLink(state, { server: GH, agent: 'cursor' }, NOW)
    const configWriteOp = first.ops.find(
      (op) => op.kind === 'writeFile' && op.path === '/tmp/ws/cursor.json',
    )
    expect(configWriteOp).toBeDefined()

    const nextRaw =
      configWriteOp?.kind === 'writeFile' ? configWriteOp.content : ''
    const afterState = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
        },
      }),
      { agents: [agentFile('cursor', '/tmp/ws/cursor.json', nextRaw)] },
    )
    const second = planLink(afterState, { server: GH, agent: 'cursor' }, NOW)
    const secondConfigWrites = second.ops.filter(
      (op) => op.kind === 'writeFile' && op.path === '/tmp/ws/cursor.json',
    )
    expect(secondConfigWrites).toHaveLength(0)
  })

  // -----------------------------------------------------------------
  // Install-status gate. Throws AgentNotInstalledError when the agent's
  // config path is under a non-existent directory. Predicts what link()
  // would throw and pairs with isInstalled({agents}) as its precheck.
  // -----------------------------------------------------------------

  test('throws AgentNotInstalledError when neither the config file nor its parent directory exists', () => {
    const state = baseState({
      agents: [agentFile('cursor', '/tmp/ws/cursor.json', '', 'system', false)],
    })
    expect(() => planLink(state, { server: GH, agent: 'cursor' }, NOW)).toThrow(
      AgentNotInstalledError,
    )
  })

  test('does NOT throw when the parent directory exists (fresh agent, no MCP yet)', () => {
    const state = baseState({
      agents: [agentFile('cursor', '/tmp/ws/cursor.json', '', 'system', true)],
    })
    const plan = planLink(state, { server: GH, agent: 'cursor' }, NOW)
    expect(plan.created).toBe(true)
  })

  test('does NOT throw when the config file already exists', () => {
    const configJson = JSON.stringify({ mcpServers: {} })
    const state = baseState({
      agents: [
        agentFile('cursor', '/tmp/ws/cursor.json', configJson, 'system', true),
      ],
    })
    const plan = planLink(state, { server: GH, agent: 'cursor' }, NOW)
    expect(plan.created).toBe(true)
  })

  test('install-gate error carries the correct agent, configPath, and parentDir', () => {
    const state = baseState({
      agents: [
        agentFile('cursor', '/tmp/ws/nope/cursor.json', '', 'system', false),
      ],
    })
    try {
      planLink(state, { server: GH, agent: 'cursor' }, NOW)
      throw new Error('expected AgentNotInstalledError')
    } catch (err) {
      expect(err).toBeInstanceOf(AgentNotInstalledError)
      const e = err as AgentNotInstalledError
      expect(e.agent).toBe('cursor')
      expect(e.configPath).toBe('/tmp/ws/nope/cursor.json')
      expect(e.parentDir).toBe('/tmp/ws/nope')
    }
  })

  test('UnsupportedTransportError still fires before the install gate', () => {
    // Transport validity is a property of the input; check it first
    // so a caller sending a garbage spec sees the semantic error and
    // does not have to install an agent just to see it.
    const state = baseState({
      agents: [
        agentFile('claude-desktop', '/tmp/ws/claude.json', '', 'system', false),
      ],
    })
    expect(() =>
      planLink(state, { server: GH_HTTP, agent: 'claude-desktop' }, NOW),
    ).toThrow(UnsupportedTransportError)
  })
})

// -------------------------------------------------------------------
// planUnlink
// -------------------------------------------------------------------

describe('planUnlink', () => {
  test('no-op when server not in manifest', () => {
    const plan = planUnlink(baseState(), {
      serverName: 'ghost',
      agent: 'cursor',
    })
    expect(plan.removed).toBe(false)
    expect(plan.ops).toEqual([])
  })

  test('no-op when agent has no link', () => {
    const state = stateWithServer(serverEntry())
    const plan = planUnlink(state, { serverName: 'gh', agent: 'cursor' })
    expect(plan.removed).toBe(false)
  })

  test('removes the link and rewrites the config file when the entry exists on disk', () => {
    const configJson = JSON.stringify({
      mcpServers: { gh: { command: 'gh-mcp' } },
    })
    const state = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
        },
      }),
      { agents: [agentFile('cursor', '/tmp/ws/cursor.json', configJson)] },
    )
    const plan = planUnlink(state, { serverName: 'gh', agent: 'cursor' })
    expect(plan.removed).toBe(true)
    expect(plan.nextManifest.servers.gh?.links.cursor).toBeUndefined()
    expect(plan.ops).toHaveLength(2)
  })
})

// -------------------------------------------------------------------
// planDisconnect — the #63 primitive
// -------------------------------------------------------------------

describe('planDisconnect (closes #63)', () => {
  test('unlinks a single agent and drops the manifest entry when it was the last', () => {
    const configJson = JSON.stringify({
      mcpServers: { gh: { command: 'gh-mcp' } },
    })
    const state = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
        },
      }),
      { agents: [agentFile('cursor', '/tmp/ws/cursor.json', configJson)] },
    )
    const plan = planDisconnect(state, { serverName: 'gh', agent: 'cursor' })
    expect(plan.unlinked).toBe(true)
    expect(plan.removedManifest).toBe(true)
    expect(plan.nextManifest.servers.gh).toBeUndefined()
  })

  test('DOES NOT drop the manifest entry when other agents remain linked', () => {
    const state = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
          'claude-code': { configPath: '/tmp/ws/claude.json', createdAt: NOW },
          vscode: { configPath: '/tmp/ws/vscode.json', createdAt: NOW },
          gemini: { configPath: '/tmp/ws/gemini.json', createdAt: NOW },
        },
      }),
      {
        agents: [
          agentFile(
            'cursor',
            '/tmp/ws/cursor.json',
            JSON.stringify({ mcpServers: { gh: { command: 'gh-mcp' } } }),
          ),
          agentFile(
            'claude-code',
            '/tmp/ws/claude.json',
            JSON.stringify({ mcpServers: { gh: { command: 'gh-mcp' } } }),
          ),
          agentFile(
            'vscode',
            '/tmp/ws/vscode.json',
            JSON.stringify({
              servers: { gh: { command: 'gh-mcp', type: 'stdio' } },
            }),
          ),
          agentFile(
            'gemini',
            '/tmp/ws/gemini.json',
            JSON.stringify({ mcpServers: { gh: { command: 'gh-mcp' } } }),
          ),
        ],
      },
    )
    const plan = planDisconnect(state, { serverName: 'gh', agent: 'cursor' })
    expect(plan.unlinked).toBe(true)
    expect(plan.removedManifest).toBe(false)
    const remainingEntry = plan.nextManifest.servers.gh
    expect(remainingEntry).toBeDefined()
    expect(Object.keys(remainingEntry?.links ?? {}).sort()).toEqual([
      'claude-code',
      'gemini',
      'vscode',
    ])
    const writePaths = plan.ops
      .filter((op) => op.kind === 'writeFile')
      .map((op) => (op.kind === 'writeFile' ? op.path : ''))
    expect(writePaths).toContain('/tmp/ws/cursor.json')
    expect(writePaths).toContain('/tmp/ws/manifest.json')
    expect(writePaths).not.toContain('/tmp/ws/claude.json')
    expect(writePaths).not.toContain('/tmp/ws/vscode.json')
    expect(writePaths).not.toContain('/tmp/ws/gemini.json')
  })

  test('removeIfLast: false keeps the manifest entry with empty links map', () => {
    const state = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
        },
      }),
      {
        agents: [
          agentFile(
            'cursor',
            '/tmp/ws/cursor.json',
            JSON.stringify({ mcpServers: { gh: { command: 'gh-mcp' } } }),
          ),
        ],
      },
    )
    const plan = planDisconnect(state, {
      serverName: 'gh',
      agent: 'cursor',
      removeIfLast: false,
    })
    expect(plan.removedManifest).toBe(false)
    expect(plan.nextManifest.servers.gh?.links).toEqual({})
  })

  test('no-op when the agent was never linked', () => {
    const state = stateWithServer(serverEntry())
    const plan = planDisconnect(state, { serverName: 'gh', agent: 'cursor' })
    expect(plan.unlinked).toBe(false)
    expect(plan.removedManifest).toBe(false)
    expect(plan.ops).toEqual([])
  })
})

// -------------------------------------------------------------------
// planRemove
// -------------------------------------------------------------------

describe('planRemove', () => {
  test('drops manifest entry and unlinks every currently-linked agent by default', () => {
    const state = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
          gemini: { configPath: '/tmp/ws/gemini.json', createdAt: NOW },
        },
      }),
      {
        agents: [
          agentFile(
            'cursor',
            '/tmp/ws/cursor.json',
            JSON.stringify({ mcpServers: { gh: { command: 'gh-mcp' } } }),
          ),
          agentFile(
            'gemini',
            '/tmp/ws/gemini.json',
            JSON.stringify({ mcpServers: { gh: { command: 'gh-mcp' } } }),
          ),
        ],
      },
    )
    const plan = planRemove(state, { serverName: 'gh' })
    expect(plan.removedManifest).toBe(true)
    expect(plan.unlinkedAgents.sort()).toEqual(['cursor', 'gemini'])
    expect(plan.nextManifest.servers.gh).toBeUndefined()
    const paths = plan.ops
      .filter((op) => op.kind === 'writeFile')
      .map((op) => (op.kind === 'writeFile' ? op.path : ''))
    expect(paths).toContain('/tmp/ws/cursor.json')
    expect(paths).toContain('/tmp/ws/gemini.json')
    expect(paths).toContain('/tmp/ws/manifest.json')
  })

  test('unlinkFirst: false drops the manifest entry without touching agent files', () => {
    const state = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
        },
      }),
    )
    const plan = planRemove(state, { serverName: 'gh', unlinkFirst: false })
    expect(plan.removedManifest).toBe(true)
    expect(plan.unlinkedAgents).toEqual([])
    expect(
      plan.ops.filter(
        (op) => op.kind === 'writeFile' && op.path.includes('cursor'),
      ),
    ).toHaveLength(0)
  })

  test('no-op when server not in manifest', () => {
    const plan = planRemove(baseState(), { serverName: 'ghost' })
    expect(plan.removedManifest).toBe(false)
    expect(plan.ops).toEqual([])
  })
})

// -------------------------------------------------------------------
// planRescan
// -------------------------------------------------------------------

describe('planRescan', () => {
  test('reports verified links when the on-disk entry matches', () => {
    const state = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
        },
      }),
      {
        agents: [
          agentFile(
            'cursor',
            '/tmp/ws/cursor.json',
            JSON.stringify({ mcpServers: { gh: { command: 'gh-mcp' } } }),
          ),
        ],
      },
    )
    const { rescan } = planRescan(state)
    expect(rescan.verified).toHaveLength(1)
    expect(rescan.drifted).toHaveLength(0)
    expect(rescan.missing).toHaveLength(0)
  })

  test('reports drift when config has no matching entry', () => {
    const state = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
        },
      }),
      {
        agents: [
          agentFile(
            'cursor',
            '/tmp/ws/cursor.json',
            JSON.stringify({ mcpServers: {} }),
          ),
        ],
      },
    )
    const { rescan } = planRescan(state)
    expect(rescan.drifted).toHaveLength(1)
    expect(rescan.drifted[0]?.serverName).toBe('gh')
  })

  test('reports missing when the file does not exist', () => {
    const state = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
        },
      }),
      { agents: [agentFile('cursor', '/tmp/ws/cursor.json', '')] },
    )
    const { rescan } = planRescan(state)
    expect(rescan.missing).toHaveLength(1)
  })

  test('filter by agents narrows the scan', () => {
    const state = stateWithServer(
      serverEntry({
        links: {
          cursor: { configPath: '/tmp/ws/cursor.json', createdAt: NOW },
          gemini: { configPath: '/tmp/ws/gemini.json', createdAt: NOW },
        },
      }),
      {
        agents: [
          agentFile(
            'cursor',
            '/tmp/ws/cursor.json',
            JSON.stringify({ mcpServers: { gh: { command: 'gh-mcp' } } }),
          ),
          agentFile(
            'gemini',
            '/tmp/ws/gemini.json',
            JSON.stringify({ mcpServers: { gh: { command: 'gh-mcp' } } }),
          ),
        ],
      },
    )
    const { rescan } = planRescan(state, { agents: ['cursor'] })
    expect(rescan.verified).toHaveLength(1)
    expect(rescan.verified[0]?.agent).toBe('cursor')
  })

  test('returns no ops (rescan is read-only)', () => {
    const { ops } = planRescan(baseState())
    expect(ops).toEqual([])
  })
})

// -------------------------------------------------------------------
// Cross-cutting invariants
// -------------------------------------------------------------------

describe('planner invariants', () => {
  test('a plan is a value; calling twice on the same state returns equivalent ops', () => {
    const state = baseState({
      agents: [agentFile('cursor', '/tmp/ws/cursor.json')],
    })
    const a = planLink(state, { server: GH, agent: 'cursor' }, NOW)
    const b = planLink(state, { server: GH, agent: 'cursor' }, NOW)
    expect(a.ops).toEqual(b.ops)
    expect(a.nextManifest).toEqual(b.nextManifest)
  })

  test('planners never mutate the input state', () => {
    const state = stateWithServer(serverEntry(), {
      agents: [agentFile('cursor', '/tmp/ws/cursor.json')],
    })
    const before = JSON.stringify(state.manifest)
    planLink(state, { server: GH, agent: 'cursor' }, NOW)
    expect(JSON.stringify(state.manifest)).toBe(before)
  })
})
