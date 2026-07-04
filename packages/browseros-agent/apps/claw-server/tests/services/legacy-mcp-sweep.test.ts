/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { AgentId } from 'agent-mcp-manager'
import {
  resetMcpManagerForTesting,
  setMcpManagerForTesting,
} from '../../src/lib/mcp-manager'
import type { Harness } from '../../src/routes/agents/schemas'
import {
  detectLegacyBrowserosEntries,
  healLegacyBrowserosEntries,
  sweepLegacyBrowserosEntries,
} from '../../src/services/legacy-mcp-sweep'
import { createStubMcpManager } from '../_helpers/stub-mcp-manager'

async function withTempDir<T>(run: (dir: string) => Promise<T>): Promise<T> {
  const dir = await mkdtemp(join(tmpdir(), 'legacy-mcp-sweep-'))
  try {
    return await run(dir)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
}

function browserosHttpJson(): string {
  return JSON.stringify(
    {
      otherTopLevel: { keep: true },
      mcpServers: {
        browseros: {
          url: 'http://127.0.0.1:9200/mcp',
        },
        'browseros-stdio': {
          command: 'npx',
          args: ['mcp-remote', 'http://localhost:9200/mcp'],
        },
        BrowserClaw: {
          url: 'http://127.0.0.1:9200/mcp',
          type: 'http',
        },
        other: {
          url: 'http://127.0.0.1:9999/other',
        },
      },
    },
    null,
    2,
  )
}

function codexToml(): string {
  return [
    'theme = "dark"',
    '',
    '[mcp_servers.browseros-stdio]',
    'command = "npx"',
    'args = ["mcp-remote", "http://127.0.0.1:9200/mcp"]',
    '',
    '[mcp_servers.BrowserClaw]',
    'url = "http://127.0.0.1:9200/mcp"',
    '',
    '[mcp_servers.foo]',
    'url = "https://example.com/mcp"',
    '',
    '[profiles.default]',
    'model = "gpt-5-codex"',
    '',
  ].join('\n')
}

beforeEach(() => {
  resetMcpManagerForTesting()
})

afterEach(() => {
  resetMcpManagerForTesting()
})

describe('sweepLegacyBrowserosEntries', () => {
  it('removes only safe legacy BrowserOS entries from Claude Code JSON configs', async () => {
    await withTempDir(async (dir) => {
      const configPath = join(dir, '.claude.json')
      await writeFile(configPath, browserosHttpJson(), 'utf8')
      const stub = createStubMcpManager()

      const removed = await sweepLegacyBrowserosEntries(
        'claude-code',
        configPath,
        { mgr: stub },
      )

      expect(removed.sort()).toEqual(['browseros', 'browseros-stdio'])
      expect(JSON.parse(await readFile(configPath, 'utf8'))).toEqual({
        otherTopLevel: { keep: true },
        mcpServers: {
          BrowserClaw: {
            url: 'http://127.0.0.1:9200/mcp',
            type: 'http',
          },
          other: {
            url: 'http://127.0.0.1:9999/other',
          },
        },
      })

      const afterFirst = await readFile(configPath, 'utf8')
      await expect(
        sweepLegacyBrowserosEntries('claude-code', configPath, { mgr: stub }),
      ).resolves.toEqual([])
      await expect(readFile(configPath, 'utf8')).resolves.toBe(afterFirst)
    })
  })

  it('leaves legacy-name entries untouched when the shape is foreign', async () => {
    await withTempDir(async (dir) => {
      const configPath = join(dir, '.claude.json')
      const source = JSON.stringify(
        {
          mcpServers: {
            browseros: {
              url: 'https://example.com/mcp',
            },
            'browseros-stdio': {
              command: 'node',
              args: ['foreign.js'],
            },
            BrowserClaw: {
              url: 'http://127.0.0.1:9200/mcp',
              type: 'http',
            },
          },
        },
        null,
        2,
      )
      await writeFile(configPath, source, 'utf8')

      await expect(
        sweepLegacyBrowserosEntries('claude-code', configPath, {
          mgr: createStubMcpManager(),
        }),
      ).resolves.toEqual([])
      await expect(readFile(configPath, 'utf8')).resolves.toBe(source)
    })
  })

  it('removes legacy Zed context_servers entries with injected fields', async () => {
    await withTempDir(async (dir) => {
      const configPath = join(dir, 'settings.json')
      await writeFile(
        configPath,
        JSON.stringify(
          {
            context_servers: {
              browseros: {
                source: 'custom',
                enabled: true,
                url: 'http://127.0.0.1:9200/mcp',
              },
              BrowserClaw: {
                source: 'custom',
                enabled: true,
                url: 'http://127.0.0.1:9200/mcp',
              },
            },
          },
          null,
          2,
        ),
        'utf8',
      )

      await expect(
        sweepLegacyBrowserosEntries('zed', configPath, {
          mgr: createStubMcpManager(),
        }),
      ).resolves.toEqual(['browseros'])
      expect(JSON.parse(await readFile(configPath, 'utf8'))).toEqual({
        context_servers: {
          BrowserClaw: {
            source: 'custom',
            enabled: true,
            url: 'http://127.0.0.1:9200/mcp',
          },
        },
      })
    })
  })

  it('removes only safe legacy entries from Codex TOML configs', async () => {
    await withTempDir(async (dir) => {
      const configPath = join(dir, 'config.toml')
      await writeFile(configPath, codexToml(), 'utf8')

      await expect(
        sweepLegacyBrowserosEntries('codex', configPath, {
          mgr: createStubMcpManager(),
        }),
      ).resolves.toEqual(['browseros-stdio'])

      const updated = await readFile(configPath, 'utf8')
      expect(updated).toContain('[mcp_servers.BrowserClaw]')
      expect(updated).toContain('[mcp_servers.foo]')
      expect(updated).toContain('[profiles.default]')
      expect(updated).not.toContain('browseros-stdio')

      const afterFirst = await readFile(configPath, 'utf8')
      await expect(
        sweepLegacyBrowserosEntries('codex', configPath, {
          mgr: createStubMcpManager(),
        }),
      ).resolves.toEqual([])
      await expect(readFile(configPath, 'utf8')).resolves.toBe(afterFirst)
    })
  })

  it('does not remove browseros-stdio for stdio-only harnesses', async () => {
    await withTempDir(async (dir) => {
      const configPath = join(dir, 'claude_desktop_config.json')
      const source = JSON.stringify(
        {
          mcpServers: {
            'browseros-stdio': {
              command: 'npx',
              args: ['mcp-remote', 'http://127.0.0.1:9200/mcp'],
            },
          },
        },
        null,
        2,
      )
      await writeFile(configPath, source, 'utf8')

      await expect(
        sweepLegacyBrowserosEntries('claude-desktop', configPath, {
          mgr: createStubMcpManager(),
        }),
      ).resolves.toEqual([])
      await expect(readFile(configPath, 'utf8')).resolves.toBe(source)
    })
  })

  it('uses manifest unlink/remove for safe legacy entries before direct surgery', async () => {
    await withTempDir(async (dir) => {
      const configPath = join(dir, '.claude.json')
      await writeFile(
        configPath,
        JSON.stringify(
          {
            mcpServers: {
              browseros: {
                url: 'http://127.0.0.1:9200/mcp',
              },
            },
          },
          null,
          2,
        ),
        'utf8',
      )
      const stub = createStubMcpManager()

      await expect(
        sweepLegacyBrowserosEntries('claude-code', configPath, { mgr: stub }),
      ).resolves.toEqual(['browseros'])

      expect(stub.calls.map((call) => call.method)).toContain('unlink')
      expect(stub.calls.map((call) => call.method)).toContain('remove')
      expect(JSON.parse(await readFile(configPath, 'utf8')).mcpServers).toEqual(
        {},
      )
    })
  })

  it('no-ops for missing and invalid config files', async () => {
    await withTempDir(async (dir) => {
      const missing = join(dir, 'missing.json')
      await expect(
        detectLegacyBrowserosEntries('claude-code', missing),
      ).resolves.toEqual([])
      await expect(
        sweepLegacyBrowserosEntries('claude-code', missing, {
          mgr: createStubMcpManager(),
        }),
      ).resolves.toEqual([])

      const invalid = join(dir, '.claude.json')
      await writeFile(invalid, '{"mcpServers":', 'utf8')
      await expect(
        sweepLegacyBrowserosEntries('claude-code', invalid, {
          mgr: createStubMcpManager(),
        }),
      ).resolves.toEqual([])
      await expect(readFile(invalid, 'utf8')).resolves.toBe('{"mcpServers":')
    })
  })
})

describe('healLegacyBrowserosEntries', () => {
  it('connects only harnesses with safe legacy entries and converges JSON and TOML configs', async () => {
    await withTempDir(async (dir) => {
      const claudePath = join(dir, '.claude.json')
      const codexPath = join(dir, 'config.toml')
      const cursorPath = join(dir, 'cursor.json')
      await writeFile(claudePath, browserosHttpJson(), 'utf8')
      await writeFile(codexPath, codexToml(), 'utf8')
      const cleanCursor = JSON.stringify(
        {
          mcpServers: {
            BrowserClaw: {
              url: 'http://127.0.0.1:9200/mcp',
            },
          },
        },
        null,
        2,
      )
      await writeFile(cursorPath, cleanCursor, 'utf8')
      const cleanMtime = (await stat(cursorPath)).mtimeMs
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const connects: Harness[] = []
      const pathByAgent: Partial<Record<AgentId, string>> = {
        'claude-code': claudePath,
        codex: codexPath,
        cursor: cursorPath,
      }

      const summary = await healLegacyBrowserosEntries({
        harnessToAgentId: {
          'Claude Code': 'claude-code',
          Codex: 'codex',
          Cursor: 'cursor',
        } as Partial<Record<Harness, AgentId | null>>,
        resolveConfigPath: async (agent) => pathByAgent[agent] ?? '',
        connect: async (harness) => {
          connects.push(harness)
          if (harness === 'Claude Code') {
            const parsed = JSON.parse(await readFile(claudePath, 'utf8'))
            parsed.mcpServers.BrowserClaw = {
              url: 'http://127.0.0.1:9200/mcp',
              type: 'http',
            }
            await writeFile(claudePath, JSON.stringify(parsed, null, 2), 'utf8')
            await sweepLegacyBrowserosEntries('claude-code', claudePath, {
              mgr: stub,
            })
          }
          if (harness === 'Codex') {
            await sweepLegacyBrowserosEntries('codex', codexPath, { mgr: stub })
          }
          return {
            harness,
            installed: true,
            agentId: pathByAgent['claude-code'] ? 'claude-code' : null,
            message: 'connected',
          }
        },
      })

      expect(connects.sort()).toEqual(['Claude Code', 'Codex'])
      expect(summary).toEqual({
        'Claude Code': ['browseros', 'browseros-stdio'],
        Codex: ['browseros-stdio'],
      })
      expect(JSON.parse(await readFile(claudePath, 'utf8')).mcpServers).toEqual(
        {
          BrowserClaw: {
            url: 'http://127.0.0.1:9200/mcp',
            type: 'http',
          },
          other: {
            url: 'http://127.0.0.1:9999/other',
          },
        },
      )
      expect(await readFile(codexPath, 'utf8')).not.toContain('browseros-stdio')
      await expect(readFile(cursorPath, 'utf8')).resolves.toBe(cleanCursor)
      expect((await stat(cursorPath)).mtimeMs).toBe(cleanMtime)
    })
  })
})
