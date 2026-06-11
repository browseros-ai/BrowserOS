/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
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

  it('hides agents BrowserOS does not surface in the panel', async () => {
    // Today only Gemini CLI is hidden: its HTTP MCP support is not
    // stable enough for a one-click install. The agent stays
    // available via the manual setup snippet.
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
    ]
    const { manager: hiddenManager } = makeManagerStub()
    setMcpManagerForTesting(hiddenManager)

    const hiddenRows = await listAgents({ detect: stubDetect })
    expect(hiddenRows.map((r) => r.id)).toEqual(['claude-code'])
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
    expect(calls.add[0].spec).toEqual({
      transport: 'http',
      url: 'http://127.0.0.1:9100/mcp',
    })
    expect(calls.link).toHaveLength(1)
    expect(calls.link[0].agent).toBe('claude-code')
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
  it('calls unlink and returns success on the happy path', async () => {
    const { manager, calls } = makeManagerStub()
    setMcpManagerForTesting(manager)
    const out = await uninstallFrom('claude-code')
    expect(out.success).toBe(true)
    expect(calls.unlink).toHaveLength(1)
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
