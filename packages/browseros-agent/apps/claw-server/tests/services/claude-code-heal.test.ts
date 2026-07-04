/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { InstalledServer, McpServerLink } from 'agent-mcp-manager'
import {
  resetMcpManagerForTesting,
  setMcpManagerForTesting,
} from '../../src/lib/mcp-manager'
import { healClaudeCodeTransportTags } from '../../src/services/claude-code-heal'
import { createStubMcpManager } from '../_helpers/stub-mcp-manager'

async function withTempConfig<T>(
  run: (configPath: string) => Promise<T>,
): Promise<T> {
  const dir = await mkdtemp(join(tmpdir(), 'claude-code-heal-'))
  try {
    return await run(join(dir, '.claude.json'))
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
}

function installManifest(
  servers: InstalledServer[],
  links: McpServerLink[],
): void {
  const stub = createStubMcpManager()
  stub.listServers = async () => servers
  stub.listLinks = async () => links
  setMcpManagerForTesting(stub)
}

function server(name: string, spec: InstalledServer['spec']): InstalledServer {
  return {
    name,
    spec,
    addedAt: '2026-07-04T00:00:00.000Z',
    links: {},
  }
}

beforeEach(() => {
  resetMcpManagerForTesting()
})

afterEach(() => {
  resetMcpManagerForTesting()
})

describe('healClaudeCodeTransportTags', () => {
  it('repairs BrowserClaw and profile claude-code http entries without tagging legacy aliases', async () => {
    await withTempConfig(async (configPath) => {
      await writeFile(
        configPath,
        JSON.stringify(
          {
            mcpServers: {
              BrowserClaw: {
                url: 'http://127.0.0.1:9200/mcp',
              },
              browseros: {
                url: 'http://127.0.0.1:9200/mcp',
              },
              'profile-one': {
                url: 'http://127.0.0.1:9200/profile-one/mcp',
              },
              'stdio-profile': {
                command: 'node',
              },
              'cursor-profile': {
                url: 'http://127.0.0.1:9200/cursor/mcp',
              },
            },
          },
          null,
          2,
        ),
        'utf8',
      )
      installManifest(
        [
          server('BrowserClaw', {
            transport: 'http',
            url: 'http://127.0.0.1:9200/mcp',
          }),
          server('profile-one', {
            transport: 'http',
            url: 'http://127.0.0.1:9200/profile-one/mcp',
          }),
          server('stdio-profile', {
            transport: 'stdio',
            command: 'node',
          }),
          server('cursor-profile', {
            transport: 'http',
            url: 'http://127.0.0.1:9200/cursor/mcp',
          }),
        ],
        [
          {
            serverName: 'BrowserClaw',
            agent: 'claude-code',
            configPath,
          },
          {
            serverName: 'profile-one',
            agent: 'claude-code',
            configPath,
          },
          {
            serverName: 'stdio-profile',
            agent: 'claude-code',
            configPath,
          },
          {
            serverName: 'cursor-profile',
            agent: 'cursor',
            configPath,
          },
        ],
      )

      await expect(healClaudeCodeTransportTags()).resolves.toBe(2)

      expect(JSON.parse(await readFile(configPath, 'utf8')).mcpServers).toEqual(
        {
          BrowserClaw: {
            url: 'http://127.0.0.1:9200/mcp',
            type: 'http',
          },
          browseros: {
            url: 'http://127.0.0.1:9200/mcp',
          },
          'profile-one': {
            url: 'http://127.0.0.1:9200/profile-one/mcp',
            type: 'http',
          },
          'stdio-profile': {
            command: 'node',
          },
          'cursor-profile': {
            url: 'http://127.0.0.1:9200/cursor/mcp',
          },
        },
      )
    })
  })

  it('skips missing config files without throwing', async () => {
    await withTempConfig(async (configPath) => {
      installManifest(
        [
          server('BrowserClaw', {
            transport: 'http',
            url: 'http://127.0.0.1:9200/mcp',
          }),
        ],
        [
          {
            serverName: 'BrowserClaw',
            agent: 'claude-code',
            configPath,
          },
        ],
      )

      await expect(healClaudeCodeTransportTags()).resolves.toBe(0)
    })
  })

  it('does not rewrite a foreign legacy browseros entry while healing BrowserClaw', async () => {
    await withTempConfig(async (configPath) => {
      await writeFile(
        configPath,
        JSON.stringify(
          {
            mcpServers: {
              BrowserClaw: {
                url: 'http://127.0.0.1:9200/mcp',
              },
              browseros: {
                command: 'node',
                args: ['foreign-server.js'],
              },
            },
          },
          null,
          2,
        ),
        'utf8',
      )
      installManifest(
        [
          server('BrowserClaw', {
            transport: 'http',
            url: 'http://127.0.0.1:9200/mcp',
          }),
        ],
        [
          {
            serverName: 'BrowserClaw',
            agent: 'claude-code',
            configPath,
          },
        ],
      )

      await expect(healClaudeCodeTransportTags()).resolves.toBe(1)

      expect(JSON.parse(await readFile(configPath, 'utf8')).mcpServers).toEqual(
        {
          BrowserClaw: {
            url: 'http://127.0.0.1:9200/mcp',
            type: 'http',
          },
          browseros: {
            command: 'node',
            args: ['foreign-server.js'],
          },
        },
      )
    })
  })

  it('is a steady-state no-op for already-tagged entries', async () => {
    await withTempConfig(async (configPath) => {
      const source = JSON.stringify(
        {
          mcpServers: {
            BrowserClaw: {
              url: 'http://127.0.0.1:9200/mcp',
              type: 'http',
            },
            browseros: {
              url: 'http://127.0.0.1:9200/mcp',
              type: 'http',
            },
          },
        },
        null,
        2,
      )
      await writeFile(configPath, source, 'utf8')
      installManifest(
        [
          server('BrowserClaw', {
            transport: 'http',
            url: 'http://127.0.0.1:9200/mcp',
          }),
        ],
        [
          {
            serverName: 'BrowserClaw',
            agent: 'claude-code',
            configPath,
          },
        ],
      )

      await expect(healClaudeCodeTransportTags()).resolves.toBe(0)
      await expect(readFile(configPath, 'utf8')).resolves.toBe(source)
    })
  })
})
