/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { setMcpManagerForTesting } from '../../src/lib/mcp-manager'
import { migrateMcpUrls } from '../../src/lib/migrate-mcp-urls'
import { readJson, writeJson } from '../../src/lib/storage'
import { storedAgentProfileSchema } from '../../src/routes/agents/schemas'
import { writeAgentProfile } from '../_helpers/agent-profile'
import { createStubMcpManager } from '../_helpers/stub-mcp-manager'
import { withTempBrowserClawDir } from '../_helpers/temp-browserclaw-dir'

describe('migrateMcpUrls', () => {
  test('relinks every manifest entry whose spec URL has moved', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      const oldUrl = 'http://127.0.0.1:8080/mcp'
      const newUrl = 'http://127.0.0.1:9200/mcp'
      // Seed the manifest with a shared BrowserClaw entry pointing
      // at the OLD url, linked to two agents. The migration must
      // relink both.
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
      // Two agents relinked; no profile files present so no profile
      // rewrites factor in.
      expect(result.migrated).toBe(2)
      expect(result.failed).toBe(0)
      // Manifest spec URL is now new.
      const servers = await stub.list()
      const bc = servers.find((s) => s.name === 'BrowserClaw')
      expect(bc?.spec).toMatchObject({ transport: 'http', url: newUrl })
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
      // No link calls fired for the unchanged entry.
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

  test('rewrites the mcpUrl field on stored profile JSON files', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const created = await writeAgentProfile({ name: 'Cowork' })
      const oldUrl = 'http://127.0.0.1:8080/mcp'
      const storedBefore = await readJson(
        `agents/${created.id}.json`,
        storedAgentProfileSchema,
      )
      await writeJson(
        `agents/${created.id}.json`,
        { ...storedBefore, mcpUrl: oldUrl },
        storedAgentProfileSchema,
      )
      await migrateMcpUrls('http://127.0.0.1:9200/mcp')
      const stored = await readJson(
        `agents/${created.id}.json`,
        storedAgentProfileSchema,
      )
      expect(stored.mcpUrl).toBe('http://127.0.0.1:9200/mcp')
    })
  })

  test('a corrupt profile file is logged + skipped without aborting the sweep', async () => {
    await withTempBrowserClawDir(async (dir) => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const ok = await writeAgentProfile({ name: 'Healthy' })
      await writeFile(
        join(dir, 'agents', 'broken.json'),
        '{ this is not valid json',
        'utf8',
      )
      const result = await migrateMcpUrls('http://127.0.0.1:9100/mcp')
      expect(result.failed).toBeGreaterThanOrEqual(1)
      const stored = await readJson(
        `agents/${ok.id}.json`,
        storedAgentProfileSchema,
      )
      expect(stored.mcpUrl).toBe('http://127.0.0.1:9100/mcp')
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
