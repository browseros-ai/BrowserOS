/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type {
  AddServerOptions,
  AgentId,
  InstalledServer,
  LinkServerOptions,
  McpManager,
  McpServerLink,
  McpServerSpec,
  RemoveServerOptions,
  UnlinkServerOptions,
} from 'agent-mcp-manager'
import {
  resetMcpManagerForTesting,
  setMcpManagerForTesting,
} from '../../src/lib/mcp-manager'
import { relinkManagedServer } from '../../src/services/mcp-relink'

interface LinkWritingManager extends McpManager {
  calls: string[]
  setNextLinkError(error: Error): void
}

async function withTempConfig<T>(
  run: (configPath: string) => Promise<T>,
): Promise<T> {
  const dir = await mkdtemp(join(tmpdir(), 'mcp-relink-'))
  try {
    return await run(join(dir, '.claude.json'))
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
}

function makeManager(options: {
  configPath: string
  existingLink?: McpServerLink
  initialServers?: InstalledServer[]
  writeOnLink?: boolean
}): LinkWritingManager {
  const specs = new Map<string, McpServerSpec>()
  for (const server of options.initialServers ?? []) {
    specs.set(server.name, server.spec)
  }
  let links = options.existingLink ? [options.existingLink] : []
  let nextLinkError: Error | null = null
  const calls: string[] = []
  const writeOnLink = options.writeOnLink ?? true

  const manager: LinkWritingManager = {
    calls,
    setNextLinkError(error: Error): void {
      nextLinkError = error
    },
    async add(opts: AddServerOptions) {
      calls.push('add')
      specs.set(opts.name, opts.spec)
      return { name: opts.name, created: true }
    },
    async link(opts: LinkServerOptions) {
      calls.push('link')
      if (nextLinkError) {
        const error = nextLinkError
        nextLinkError = null
        throw error
      }
      const configPath = opts.configPath ?? options.configPath
      if (writeOnLink) {
        const spec = specs.get(opts.serverName)
        await writeBareConfig(configPath, opts.serverName, spec)
      }
      links = [
        {
          serverName: opts.serverName,
          agent: opts.agent,
          configPath,
        },
      ]
      return {
        serverName: opts.serverName,
        agent: opts.agent,
        configPath,
        created: true,
      }
    },
    async unlink(opts: UnlinkServerOptions) {
      calls.push('unlink')
      links = links.filter(
        (link) =>
          link.serverName !== opts.serverName || link.agent !== opts.agent,
      )
      return {
        serverName: opts.serverName,
        agent: opts.agent,
        configPath: opts.configPath ?? options.configPath,
        removed: true,
      }
    },
    async remove(_opts: RemoveServerOptions) {
      calls.push('remove')
    },
    async listServers() {
      calls.push('listServers')
      return Array.from(specs, ([name, spec]) => ({
        name,
        spec,
        addedAt: '2026-07-04T00:00:00.000Z',
        links: {},
      }))
    },
    async listLinks() {
      calls.push('listLinks')
      return links
    },
    async rescan() {
      calls.push('rescan')
      return { verified: [], drifted: [], broken: [], unmanaged: [] }
    },
  }
  return manager
}

async function writeBareConfig(
  configPath: string,
  serverName: string,
  spec: McpServerSpec | undefined,
): Promise<void> {
  const entry =
    spec?.transport === 'http'
      ? { url: spec.url }
      : { command: 'node', args: ['server.js'] }
  let existing: Record<string, unknown> = {}
  try {
    existing = JSON.parse(await readFile(configPath, 'utf8'))
  } catch {
    existing = {}
  }
  const existingServers =
    typeof existing.mcpServers === 'object' && existing.mcpServers !== null
      ? existing.mcpServers
      : {}
  await writeFile(
    configPath,
    JSON.stringify(
      { ...existing, mcpServers: { ...existingServers, [serverName]: entry } },
      null,
      2,
    ),
    'utf8',
  )
}

async function relinkWith(
  manager: McpManager,
  agent: AgentId,
  spec: McpServerSpec,
) {
  return await relinkManagedServer({
    mgr: manager,
    serverName: 'BrowserClaw',
    agent,
    spec,
  })
}

describe('relinkManagedServer', () => {
  beforeEach(() => {
    resetMcpManagerForTesting()
  })

  afterEach(() => {
    resetMcpManagerForTesting()
  })

  it('tags claude-code http links and converges on repeated relinks', async () => {
    await withTempConfig(async (configPath) => {
      const manager = makeManager({ configPath })
      const spec = {
        transport: 'http' as const,
        url: 'http://127.0.0.1:9200/mcp',
      }

      await relinkWith(manager, 'claude-code', spec)
      const first = await readFile(configPath, 'utf8')
      expect(JSON.parse(first).mcpServers.BrowserClaw).toEqual({
        url: 'http://127.0.0.1:9200/mcp',
        type: 'http',
      })

      await relinkWith(manager, 'claude-code', spec)
      await expect(readFile(configPath, 'utf8')).resolves.toBe(first)
    })
  })

  it('sweeps safe legacy BrowserOS aliases after writing the canonical Claude Code entry', async () => {
    await withTempConfig(async (configPath) => {
      await writeFile(
        configPath,
        JSON.stringify(
          {
            mcpServers: {
              browseros: {
                url: 'http://127.0.0.1:9200/mcp',
              },
              'browseros-stdio': {
                command: 'npx',
                args: ['mcp-remote', 'http://127.0.0.1:9200/mcp'],
              },
              BrowserClaw: {
                url: 'http://127.0.0.1:9200/mcp',
              },
            },
          },
          null,
          2,
        ),
        'utf8',
      )
      const manager = makeManager({ configPath })
      setMcpManagerForTesting(manager)

      await relinkWith(manager, 'claude-code', {
        transport: 'http',
        url: 'http://127.0.0.1:9200/mcp',
      })

      expect(JSON.parse(await readFile(configPath, 'utf8')).mcpServers).toEqual(
        {
          BrowserClaw: {
            url: 'http://127.0.0.1:9200/mcp',
            type: 'http',
          },
        },
      )
    })
  })

  it('does not tag claude-code stdio links', async () => {
    await withTempConfig(async (configPath) => {
      const manager = makeManager({ configPath })

      await relinkWith(manager, 'claude-code', {
        transport: 'stdio',
        command: 'node',
        args: ['server.js'],
      })

      expect(JSON.parse(await readFile(configPath, 'utf8')).mcpServers).toEqual(
        {
          BrowserClaw: {
            command: 'node',
            args: ['server.js'],
          },
        },
      )
    })
  })

  it('does not tag non-claude-code http links', async () => {
    await withTempConfig(async (configPath) => {
      const manager = makeManager({ configPath })

      await relinkWith(manager, 'cursor', {
        transport: 'http',
        url: 'http://127.0.0.1:9200/mcp',
      })

      expect(JSON.parse(await readFile(configPath, 'utf8')).mcpServers).toEqual(
        {
          BrowserClaw: {
            url: 'http://127.0.0.1:9200/mcp',
          },
        },
      )
    })
  })

  it('tags the restored claude-code http entry on rollback', async () => {
    await withTempConfig(async (configPath) => {
      const previousSpec = {
        transport: 'http' as const,
        url: 'http://127.0.0.1:9200/mcp',
      }
      const manager = makeManager({
        configPath,
        existingLink: {
          serverName: 'BrowserClaw',
          agent: 'claude-code',
          configPath,
        },
        initialServers: [
          {
            name: 'BrowserClaw',
            spec: previousSpec,
            addedAt: '2026-07-04T00:00:00.000Z',
            links: {},
          },
        ],
      })
      manager.setNextLinkError(new Error('write denied'))

      await expect(
        relinkWith(manager, 'claude-code', {
          transport: 'http',
          url: 'http://127.0.0.1:9512/mcp',
        }),
      ).rejects.toThrow('write denied')

      expect(JSON.parse(await readFile(configPath, 'utf8')).mcpServers).toEqual(
        {
          BrowserClaw: {
            url: 'http://127.0.0.1:9200/mcp',
            type: 'http',
          },
        },
      )
    })
  })

  it('does not fail a successful relink when tagging cannot read the config path', async () => {
    const dir = await mkdtemp(join(tmpdir(), 'mcp-relink-unreadable-'))
    try {
      const manager = makeManager({
        configPath: dir,
        writeOnLink: false,
      })

      await expect(
        relinkWith(manager, 'claude-code', {
          transport: 'http',
          url: 'http://127.0.0.1:9200/mcp',
        }),
      ).resolves.toMatchObject({
        serverName: 'BrowserClaw',
        agent: 'claude-code',
        configPath: dir,
      })
    } finally {
      await rm(dir, { recursive: true, force: true })
    }
  })
})
