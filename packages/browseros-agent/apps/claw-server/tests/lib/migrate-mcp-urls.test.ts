/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises'
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

  test('does NOT advance the stored mcpUrl when the manifest relink for that slug fails', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      // Seed a profile whose slug lives in the manifest with the old URL.
      const created = await writeAgentProfile({ name: 'Retry Install' })
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
      await stub.link({
        server: {
          name: created.slug,
          spec: { transport: 'http', url: oldUrl },
        },
        agent: 'claude-code',
      })
      // Every mgr.link fails. The invariant: profile JSON must stay
      // on the OLD URL so the next boot retries against a consistent
      // manifest / profile pair, not diverge into a three-way split.
      stub.link = async () => {
        throw new Error('link boom')
      }
      setMcpManagerForTesting(stub)
      const result = await migrateMcpUrls('http://127.0.0.1:9200/mcp')
      expect(result.failed).toBeGreaterThan(0)
      const stored = await readJson(
        `agents/${created.id}.json`,
        storedAgentProfileSchema,
      )
      expect(stored.mcpUrl).toBe(oldUrl)
    })
  })

  test('quarantines a profile whose harness value is no longer in the enum', async () => {
    await withTempBrowserClawDir(async (root) => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      // Write a retired-harness profile directly (bypasses zod so we
      // can plant a `harness: 'Claude Desktop'` value the current
      // enum rejects).
      const retiredProfile = {
        id: 'oldie',
        slug: 'oldie',
        name: 'Old Profile',
        harness: 'Claude Desktop',
        loginMode: 'profile',
        selectedSites: [],
        approvals: {},
        aclRuleIds: [],
        customAclRules: [],
        status: 'configured',
        mcpUrl: 'http://127.0.0.1:9200/mcp',
        createdAt: '2026-01-01T00:00:00Z',
        updatedAt: '2026-01-01T00:00:00Z',
      }
      await mkdir(join(root, 'agents'), { recursive: true })
      await writeFile(
        join(root, 'agents', 'oldie.json'),
        JSON.stringify(retiredProfile),
        'utf8',
      )
      const result = await migrateMcpUrls('http://127.0.0.1:9100/mcp')
      // Retired-harness quarantine does NOT count as `failed`; it is
      // a clean routed outcome. Verify the file moved.
      expect(result.failed).toBe(0)
      const originalGone = await stat(join(root, 'agents', 'oldie.json')).catch(
        () => null,
      )
      expect(originalGone).toBeNull()
      const raw = await readFile(
        join(root, 'agents', '.retired', 'oldie.json'),
        'utf8',
      )
      expect(JSON.parse(raw).harness).toBe('Claude Desktop')
    })
  })

  test('a corrupt profile file (unparseable JSON) is logged + skipped without aborting the sweep', async () => {
    await withTempBrowserClawDir(async (root) => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const ok = await writeAgentProfile({ name: 'Healthy' })
      await writeFile(
        join(root, 'agents', 'broken.json'),
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
