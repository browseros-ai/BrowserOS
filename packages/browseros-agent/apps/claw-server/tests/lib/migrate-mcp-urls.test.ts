/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { readFile, stat, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { setMcpManagerForTesting } from '../../src/lib/mcp-manager'
import { migrateMcpUrls } from '../../src/lib/migrate-mcp-urls'
import { createStubMcpManager } from '../_helpers/stub-mcp-manager'
import { withTempBrowserClawDir } from '../_helpers/temp-browserclaw-dir'

describe('migrateMcpUrls', () => {
  test('ignores manifest entries outside the shared BrowserClaw server', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      const oldUrl = 'http://127.0.0.1:8080/mcp'
      await stub.link({
        server: {
          name: 'legacy-profile-slug',
          spec: { transport: 'http', url: oldUrl },
        },
        agent: 'claude-code',
      })
      setMcpManagerForTesting(stub)
      stub.reset()

      const result = await migrateMcpUrls('http://127.0.0.1:9200/mcp')

      expect(result).toEqual({ migrated: 0, skipped: 0, failed: 0 })
      expect(stub.calls.filter((c) => c.method === 'link')).toHaveLength(0)
      const servers = await stub.list()
      expect(servers[0]?.spec).toEqual({ transport: 'http', url: oldUrl })
    })
  })

  test('does not inspect the legacy agents path', async () => {
    await withTempBrowserClawDir(async (root) => {
      const sentinel = join(root, 'agents')
      await writeFile(sentinel, 'untouched', 'utf8')
      const stub = createStubMcpManager()
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: 'http://127.0.0.1:8080/mcp' },
        },
        agent: 'cursor',
      })
      setMcpManagerForTesting(stub)
      stub.reset()

      const result = await migrateMcpUrls('http://127.0.0.1:9200/mcp')

      expect(result).toEqual({ migrated: 1, skipped: 0, failed: 0 })
      expect(await readFile(sentinel, 'utf8')).toBe('untouched')
    })
  })

  test('relinks every manifest entry whose spec URL has moved', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      const oldUrl = 'http://127.0.0.1:8080/mcp'
      const newUrl = 'http://127.0.0.1:9200/mcp'
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: oldUrl },
        },
        agent: 'claude-code',
      })
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: oldUrl },
        },
        agent: 'cursor',
      })
      setMcpManagerForTesting(stub)
      stub.reset()
      const result = await migrateMcpUrls(newUrl)
      expect(result.migrated).toBe(2)
      expect(result.failed).toBe(0)
      const servers = await stub.list()
      const bc = servers.find((s) => s.name === 'BrowserClaw')
      expect(bc?.spec).toMatchObject({ transport: 'http', url: newUrl })
    })
  })

  test('relinks the exact config path recorded in the manifest', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      const configPath = '/tmp/custom-claude-mcp.json'
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: 'http://127.0.0.1:8080/mcp' },
        },
        agent: 'claude-code',
        configPath,
      })
      setMcpManagerForTesting(stub)
      stub.reset()

      const result = await migrateMcpUrls('http://127.0.0.1:9200/mcp')

      expect(result).toEqual({ migrated: 1, skipped: 0, failed: 0 })
      const relink = stub.calls.find((call) => call.method === 'link')
      expect(relink?.payload).toMatchObject({
        agent: 'claude-code',
        configPath,
      })
      const [server] = await stub.list()
      expect(server?.links['claude-code']?.configPath).toBe(configPath)
    })
  })

  test('keeps a pending marker so the next boot retries every harness', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      const oldUrl = 'http://127.0.0.1:8080/mcp'
      const newUrl = 'http://127.0.0.1:9200/mcp'
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: oldUrl },
        },
        agent: 'claude-code',
      })
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: oldUrl },
        },
        agent: 'cursor',
      })
      const originalLink = stub.link
      let failClaude = true
      stub.link = async (input) => {
        if (failClaude && input.agent === 'claude-code') {
          throw new Error('config locked')
        }
        return originalLink(input)
      }
      setMcpManagerForTesting(stub)
      stub.reset()

      const firstResult = await migrateMcpUrls(newUrl)

      expect(firstResult).toEqual({ migrated: 1, skipped: 0, failed: 1 })
      let servers = await stub.list()
      let bc = servers.find((s) => s.name === 'BrowserClaw')
      expect(bc?.spec).toMatchObject({ transport: 'http', url: newUrl })

      failClaude = false
      stub.reset()
      const secondResult = await migrateMcpUrls(newUrl)

      expect(secondResult).toEqual({ migrated: 2, skipped: 0, failed: 0 })
      servers = await stub.list()
      bc = servers.find((s) => s.name === 'BrowserClaw')
      expect(bc?.spec).toMatchObject({ transport: 'http', url: newUrl })
    })
  })

  test('replays every link when a prior run was interrupted', async () => {
    await withTempBrowserClawDir(async (root) => {
      const pendingFile = join(root, 'mcp-url-migration.pending')
      await writeFile(pendingFile, 'interrupted', 'utf8')
      const stub = createStubMcpManager()
      const url = 'http://127.0.0.1:9200/mcp'
      await stub.link({
        server: { name: 'BrowserClaw', spec: { transport: 'http', url } },
        agent: 'claude-code',
      })
      await stub.link({
        server: { name: 'BrowserClaw', spec: { transport: 'http', url } },
        agent: 'cursor',
      })
      setMcpManagerForTesting(stub)
      stub.reset()

      const result = await migrateMcpUrls(url)

      expect(result).toEqual({ migrated: 2, skipped: 0, failed: 0 })
      expect(stub.calls.filter((c) => c.method === 'link')).toHaveLength(2)
      const pendingStillExists = await stat(pendingFile)
        .then(() => true)
        .catch(() => false)
      expect(pendingStillExists).toBe(false)
    })
  })

  test('skips a server whose spec URL already matches the target', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      const url = 'http://127.0.0.1:9200/mcp'
      await stub.link({
        server: { name: 'BrowserClaw', spec: { transport: 'http', url } },
        agent: 'cursor',
      })
      setMcpManagerForTesting(stub)
      stub.reset()
      const result = await migrateMcpUrls(url)
      expect(result.migrated).toBe(0)
      expect(result.skipped).toBe(1)
      expect(stub.calls.filter((c) => c.method === 'link')).toHaveLength(0)
    })
  })

  test('rewrites the URL inside stdio args (npx mcp-remote wrapping)', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      const oldUrl = 'http://127.0.0.1:8080/mcp'
      const newUrl = 'http://127.0.0.1:9200/mcp'
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: {
            transport: 'stdio',
            command: 'npx',
            args: ['mcp-remote', oldUrl],
          },
        },
        agent: 'claude-code',
      })
      setMcpManagerForTesting(stub)
      stub.reset()
      await migrateMcpUrls(newUrl)
      const servers = await stub.list()
      const bc = servers.find((s) => s.name === 'BrowserClaw')
      expect(bc?.spec).toMatchObject({
        transport: 'stdio',
        command: 'npx',
        args: ['mcp-remote', newUrl],
      })
    })
  })

  test('rewrites only the FIRST http-like stdio arg (symmetric with extractSpecUrl)', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      const oldUrl = 'http://127.0.0.1:8080/mcp'
      const newUrl = 'http://127.0.0.1:9200/mcp'
      // Contrived stdio spec carrying TWO http-shaped args: the mcp
      // URL and a separate auth URL. `extractSpecUrl` uses find()
      // to key on the first; `rewriteSpecUrl` must match, so only
      // the first is overwritten. Guarantees a future harness catalog
      // that ships this shape does not silently corrupt the second URL.
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: {
            transport: 'stdio',
            command: 'npx',
            args: ['mcp-remote', oldUrl, '--auth', 'http://auth.example/z'],
          },
        },
        agent: 'claude-code',
      })
      setMcpManagerForTesting(stub)
      stub.reset()
      await migrateMcpUrls(newUrl)
      const servers = await stub.list()
      const bc = servers.find((s) => s.name === 'BrowserClaw')
      expect(bc?.spec).toMatchObject({
        transport: 'stdio',
        command: 'npx',
        args: ['mcp-remote', newUrl, '--auth', 'http://auth.example/z'],
      })
    })
  })

  test('an empty manifest returns zero counts and does not throw', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const result = await migrateMcpUrls('http://127.0.0.1:9100/mcp')
      expect(result.migrated).toBe(0)
      expect(result.failed).toBe(0)
    })
  })
})
