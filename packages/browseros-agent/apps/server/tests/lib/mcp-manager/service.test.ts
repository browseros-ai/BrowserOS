/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  type AddServerOptions,
  type AgentInfo,
  ForeignEntryError,
  type LinkServerOptions,
  type McpManager,
  type UnlinkServerOptions,
} from 'agent-mcp-manager'
import {
  installInto,
  listAgents,
  resetMcpManagerForTesting,
  setMcpManagerForTesting,
  uninstallFrom,
} from '../../../src/lib/mcp-manager'

let stubAgents: AgentInfo[] = []
const stubDetect = async (): Promise<AgentInfo[]> => stubAgents

async function withTempMcpEnv<T>(
  run: (paths: { claudeConfigPath: string }) => Promise<T>,
): Promise<T> {
  const root = await mkdtemp(join(tmpdir(), 'mcp-manager-service-'))
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
    return await run({ claudeConfigPath })
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

function makeManagerStub(
  opts: {
    links?: Array<{
      serverName: string
      agent: string
      configPath: string
    }>
    unlinkThrows?: Error
  } = {},
): {
  manager: McpManager
  calls: {
    add: AddServerOptions[]
    link: LinkServerOptions[]
    unlink: UnlinkServerOptions[]
  }
} {
  const calls = {
    add: [] as AddServerOptions[],
    link: [] as LinkServerOptions[],
    unlink: [] as UnlinkServerOptions[],
  }
  const manager: McpManager = {
    add: mock(async (o: AddServerOptions) => {
      calls.add.push(o)
      return { name: o.name, created: true }
    }),
    link: mock(async (o: LinkServerOptions) => {
      calls.link.push(o)
      return {
        serverName: o.serverName,
        agent: o.agent,
        configPath: `/tmp/fake/${o.agent}.json`,
        created: true,
      }
    }),
    unlink: mock(async (o: UnlinkServerOptions) => {
      calls.unlink.push(o)
      if (opts.unlinkThrows) throw opts.unlinkThrows
      return {
        serverName: o.serverName,
        agent: o.agent,
        configPath: `/tmp/fake/${o.agent}.json`,
        removed: true,
      }
    }),
    remove: mock(async () => {}),
    listServers: mock(async () => []),
    listLinks: mock(async () => opts.links ?? []),
    rescan: mock(async () => ({
      verified: [],
      drifted: [],
      broken: [],
      unmanaged: [],
    })),
  } as unknown as McpManager
  return { manager, calls }
}

beforeEach(() => {
  resetMcpManagerForTesting()
  stubAgents = []
})

afterEach(() => {
  resetMcpManagerForTesting()
})

describe('listAgents', () => {
  it('combines detection with manifest link state', async () => {
    stubAgents = [
      {
        id: 'claude-code',
        displayName: 'Claude Code',
        installed: true,
        configPath: '/tmp/fake/claude-code.json',
      },
      {
        id: 'cursor',
        displayName: 'Cursor',
        installed: false,
        configPath: null,
      },
    ]
    const { manager } = makeManagerStub({
      links: [
        {
          serverName: 'browseros',
          agent: 'claude-code',
          configPath: '/tmp/fake/claude-code.json',
        },
      ],
    })
    setMcpManagerForTesting(manager)

    const rows = await listAgents({ detect: stubDetect })
    expect(rows.find((r) => r.id === 'claude-code')).toMatchObject({
      installed: true,
      linked: true,
    })
    expect(rows.find((r) => r.id === 'cursor')).toMatchObject({
      installed: false,
      linked: false,
    })
  })

  it('hides agents BrowserOS does not surface when they have no active link', async () => {
    // Hidden: Gemini CLI (HTTP MCP support not stable enough for a
    // one-click install) and Claude Desktop (only stdio config is
    // valid and the recommended `npx mcp-remote` bridge needs Node
    // on the user's machine). Both stay available via the manual
    // setup snippet.
    stubAgents = [
      {
        id: 'claude-code',
        displayName: 'Claude Code',
        installed: true,
        configPath: '/tmp/fake/claude-code.json',
      },
      {
        id: 'gemini',
        displayName: 'Gemini CLI',
        installed: true,
        configPath: '/tmp/fake/gemini.json',
      },
      {
        id: 'claude-desktop',
        displayName: 'Claude Desktop',
        installed: true,
        configPath: '/tmp/fake/claude-desktop.json',
      },
    ]
    const { manager: hiddenManager } = makeManagerStub()
    setMcpManagerForTesting(hiddenManager)

    const hiddenRows = await listAgents({ detect: stubDetect })
    expect(hiddenRows.map((r) => r.id)).toEqual(['claude-code'])
  })

  it('keeps a hidden agent visible while it still has a BrowserOS link so the user can Disconnect it', async () => {
    // Regression: when we hid Claude Desktop, users who had already
    // linked it via a prior BrowserOS release lost the Disconnect
    // tile and were stuck with an orphan entry in their config.
    // The hidden-agents filter must respect existing links.
    stubAgents = [
      {
        id: 'claude-code',
        displayName: 'Claude Code',
        installed: true,
        configPath: '/tmp/fake/claude-code.json',
      },
      {
        id: 'claude-desktop',
        displayName: 'Claude Desktop',
        installed: true,
        configPath: '/tmp/fake/claude-desktop.json',
      },
    ]
    const { manager } = makeManagerStub({
      links: [
        {
          serverName: 'browseros-stdio',
          agent: 'claude-desktop',
          configPath: '/tmp/fake/claude-desktop.json',
        },
      ],
    })
    setMcpManagerForTesting(manager)

    const rows = await listAgents({ detect: stubDetect })
    expect(rows.map((r) => r.id).sort()).toEqual([
      'claude-code',
      'claude-desktop',
    ])
    expect(rows.find((r) => r.id === 'claude-desktop')?.linked).toBe(true)
  })

  it('counts codex as linked when wired up under the stdio server name', async () => {
    stubAgents = [
      {
        id: 'codex',
        displayName: 'Codex',
        installed: true,
        configPath: '/tmp/fake/codex.toml',
      },
    ]
    const { manager } = makeManagerStub({
      links: [
        {
          serverName: 'browseros-stdio',
          agent: 'codex',
          configPath: '/tmp/fake/codex.toml',
        },
      ],
    })
    setMcpManagerForTesting(manager)
    const rows = await listAgents({ detect: stubDetect })
    expect(rows.find((r) => r.id === 'codex')?.linked).toBe(true)
  })

  it('ignores manifest links to other server names', async () => {
    stubAgents = [
      {
        id: 'claude-code',
        displayName: 'Claude Code',
        installed: true,
        configPath: '/tmp/fake/claude-code.json',
      },
    ]
    const { manager } = makeManagerStub({
      links: [
        {
          serverName: 'some-other-server',
          agent: 'claude-code',
          configPath: '/tmp/fake/claude-code.json',
        },
      ],
    })
    setMcpManagerForTesting(manager)

    const rows = await listAgents({ detect: stubDetect })
    expect(rows.find((r) => r.id === 'claude-code')?.linked).toBe(false)
  })
})

describe('installInto', () => {
  it('adds the browseros entry with the current url and links the agent', async () => {
    const { manager, calls } = makeManagerStub()
    setMcpManagerForTesting(manager)

    const result = await installInto('claude-code', 'http://127.0.0.1:9100/mcp')
    expect(result.success).toBe(true)
    expect(calls.add).toHaveLength(1)
    expect(calls.add[0].name).toBe('browseros')
    expect(calls.add[0].spec).toEqual({
      transport: 'http',
      url: 'http://127.0.0.1:9100/mcp',
    })
    expect(calls.link).toHaveLength(1)
    expect(calls.link[0].agent).toBe('claude-code')
    expect(calls.link[0].serverName).toBe('browseros')
  })

  it('writes the claude-code system-scope http transport tag', async () => {
    await withTempMcpEnv(async ({ claudeConfigPath }) => {
      await writeFile(claudeConfigPath, '{"mcpServers":{}}\n', 'utf8')

      const result = await installInto(
        'claude-code',
        'http://127.0.0.1:9100/mcp',
      )

      expect(result.success).toBe(true)
      const config = JSON.parse(await readFile(claudeConfigPath, 'utf8'))
      expect(config.mcpServers.browseros).toEqual({
        url: 'http://127.0.0.1:9100/mcp',
        type: 'http',
      })
    })
  })

  it('uses an http spec under the http server name for codex', async () => {
    // Codex gained streamable-HTTP MCP support in agent-mcp-manager
    // 0.0.3 (its surface flipped to ['stdio', 'http']). planFor now
    // hits the http branch, mirroring claude-code, so no stdio bridge
    // is needed and the integration works without npx on the host.
    const { manager, calls } = makeManagerStub()
    setMcpManagerForTesting(manager)

    const result = await installInto('codex', 'http://127.0.0.1:9100/mcp')
    expect(result.success).toBe(true)
    expect(calls.add).toHaveLength(1)
    expect(calls.add[0].name).toBe('browseros')
    expect(calls.add[0].spec).toEqual({
      transport: 'http',
      url: 'http://127.0.0.1:9100/mcp',
    })
    expect(calls.link).toHaveLength(1)
    expect(calls.link[0].agent).toBe('codex')
    expect(calls.link[0].serverName).toBe('browseros')
  })

  it('uses a stdio mcp-remote spec under the stdio server name for claude-desktop', async () => {
    // Claude Desktop only accepts stdio MCP entries; an http spec is
    // silently dropped on launch. The agent-mcp-manager catalog flags
    // it as stdio-only and `planFor` honours that.
    const { manager, calls } = makeManagerStub()
    setMcpManagerForTesting(manager)

    const result = await installInto(
      'claude-desktop',
      'http://127.0.0.1:9100/mcp',
    )
    expect(result.success).toBe(true)
    expect(calls.add).toHaveLength(1)
    expect(calls.add[0].name).toBe('browseros-stdio')
    expect(calls.add[0].spec).toEqual({
      transport: 'stdio',
      command: 'npx',
      args: ['mcp-remote', 'http://127.0.0.1:9100/mcp'],
    })
    expect(calls.link).toHaveLength(1)
    expect(calls.link[0].agent).toBe('claude-desktop')
    expect(calls.link[0].serverName).toBe('browseros-stdio')
  })

  it('sweeps the opposite server name before linking, so an agent never ends up double-linked', async () => {
    // claude-desktop is stdio-only in v0.0.2; if a v0.0.1 install
    // left a legacy `browseros` (http) link for the same agent,
    // installInto must unlink it before linking under
    // `browseros-stdio`. Otherwise an uninstall click later (which
    // sweeps both names) would silently delete a "fresh" install
    // the user just clicked.
    const { manager, calls } = makeManagerStub()
    setMcpManagerForTesting(manager)
    const result = await installInto(
      'claude-desktop',
      'http://127.0.0.1:9100/mcp',
    )
    expect(result.success).toBe(true)
    expect(calls.unlink).toHaveLength(1)
    expect(calls.unlink[0].serverName).toBe('browseros')
    expect(calls.unlink[0].agent).toBe('claude-desktop')
    expect(calls.link).toHaveLength(1)
    expect(calls.link[0].serverName).toBe('browseros-stdio')
  })

  it('rejects unsupported agent ids', async () => {
    const { manager } = makeManagerStub()
    setMcpManagerForTesting(manager)
    await expect(
      installInto('not-a-real-agent', 'http://127.0.0.1:9100/mcp'),
    ).rejects.toMatchObject({ agent: 'not-a-real-agent' })
  })
})

describe('uninstallFrom', () => {
  it('sweeps both server names on uninstall and returns success', async () => {
    // The same agent can be linked under either `browseros` (http) or
    // `browseros-stdio` (mcp-remote shim) depending on which version
    // of the agent-mcp-manager catalog wrote the link. Uninstall
    // must hit both so a legacy link under the prior server name
    // cannot survive forever.
    const { manager, calls } = makeManagerStub()
    setMcpManagerForTesting(manager)
    const out = await uninstallFrom('claude-code')
    expect(out.success).toBe(true)
    expect(calls.unlink).toHaveLength(2)
    expect(calls.unlink.map((c) => c.serverName).sort()).toEqual([
      'browseros',
      'browseros-stdio',
    ])
    for (const call of calls.unlink) {
      expect(call.agent).toBe('claude-code')
    }
  })

  it('sweeps both server names for stdio-only agents too', async () => {
    const { manager, calls } = makeManagerStub()
    setMcpManagerForTesting(manager)
    const out = await uninstallFrom('codex')
    expect(out.success).toBe(true)
    expect(calls.unlink).toHaveLength(2)
    expect(calls.unlink.map((c) => c.serverName).sort()).toEqual([
      'browseros',
      'browseros-stdio',
    ])
  })

  it('uninstalls a stdio-only agent that is still linked under the legacy http server name', async () => {
    // Regression: claude-desktop was http-routed under v0.0.1; the
    // v0.0.2 catalog now classifies it as stdio-only. Without the
    // server-name sweep, uninstall would target browseros-stdio
    // (no-op) and leave the legacy `browseros` link in place
    // forever, so listAgents would keep reporting linked: true.
    const { manager, calls } = makeManagerStub()
    setMcpManagerForTesting(manager)
    const out = await uninstallFrom('claude-desktop')
    expect(out.success).toBe(true)
    expect(calls.unlink).toHaveLength(2)
    expect(calls.unlink.map((c) => c.serverName).sort()).toEqual([
      'browseros',
      'browseros-stdio',
    ])
  })

  it('returns a human message on ForeignEntryError instead of throwing', async () => {
    const { manager } = makeManagerStub({
      unlinkThrows: new ForeignEntryError(
        'browseros',
        'claude-code',
        '/tmp/fake/claude-code.json',
      ),
    })
    setMcpManagerForTesting(manager)
    const out = await uninstallFrom('claude-code')
    expect(out.success).toBe(false)
    expect(out.message).toContain('user-edited')
  })
})
