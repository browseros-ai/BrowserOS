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
import { readJson } from '../../src/lib/storage'
import type { NewAgentValues } from '../../src/routes/agents/schemas'
import { storedAgentProfileSchema } from '../../src/routes/agents/schemas'
import * as agents from '../../src/routes/agents/service'
import { createStubMcpManager } from '../_helpers/stub-mcp-manager'
import { withTempBrowserosDir } from '../_helpers/temp-browseros-dir'

function makeInput(overrides: Partial<NewAgentValues> = {}): NewAgentValues {
  return {
    name: 'Original',
    harness: 'Claude Desktop',
    loginMode: 'profile',
    selectedSites: [],
    approvals: {
      submit: 'Ask',
      payment: 'Block',
      delete: 'Ask',
      upload: 'Ask',
      navigate: 'Auto',
      input: 'Auto',
    },
    aclRuleIds: [],
    customAclRules: [],
    ...overrides,
  }
}

describe('migrateMcpUrls', () => {
  test('rewrites mcpUrl when the recomputed URL differs from the stored one', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeInput({ name: 'Cowork' }))
      // Sanity: stored mcpUrl is whatever buildMcpUrl produced at create
      // (in the test environment that's `http://127.0.0.1:9200/mcp/cowork`).
      expect(created.mcpUrl).toContain('/mcp/cowork')

      // New shape: same slug, different host + prefix.
      const newBuilder = (slug: string) =>
        `http://127.0.0.1:9100/cockpit/mcp/${slug}`
      const result = await migrateMcpUrls(newBuilder)
      expect(result.migrated).toBe(1)
      expect(result.skipped).toBe(0)
      expect(result.failed).toBe(0)

      const stored = await readJson(
        `agents/${created.id}.json`,
        storedAgentProfileSchema,
      )
      expect(stored.mcpUrl).toBe('http://127.0.0.1:9100/cockpit/mcp/cowork')
    })
  })

  test('skips a profile whose stored URL already matches the new shape', async () => {
    await withTempBrowserosDir(async () => {
      const created = await agents.create(makeInput({ name: 'Stable' }))
      const sameBuilder = (slug: string) =>
        created.mcpUrl.replace(/\/[^/]*$/, `/${slug}`)
      const result = await migrateMcpUrls(sameBuilder)
      expect(result.migrated).toBe(0)
      expect(result.skipped).toBe(1)
      expect(result.failed).toBe(0)
    })
  })

  test('re-installs the harness entry per migrated row', async () => {
    await withTempBrowserosDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const created = await agents.create(makeInput({ name: 'Reinstall' }))
      stub.reset()
      const newBuilder = (slug: string) =>
        `http://127.0.0.1:9100/cockpit/mcp/${slug}`
      await migrateMcpUrls(newBuilder)
      const methods = stub.calls.map((c) => c.method)
      // Migration: uninstall (old entry) then install (new URL).
      expect(methods).toContain('unlink')
      expect(methods).toContain('add')
      expect(methods).toContain('link')
      const addCall = stub.calls.find((c) => c.method === 'add')
      expect(addCall?.payload).toMatchObject({
        name: created.slug,
        spec: {
          transport: 'http',
          url: `http://127.0.0.1:9100/cockpit/mcp/${created.slug}`,
        },
      })
    })
  })

  test('a corrupt profile file is logged + skipped without aborting the sweep', async () => {
    await withTempBrowserosDir(async (dir) => {
      const ok = await agents.create(makeInput({ name: 'Healthy' }))
      // Drop a garbage file next to the valid one.
      await writeFile(
        join(dir, 'mcp-interface/agents', 'broken.json'),
        '{ this is not valid json',
        'utf8',
      )
      const newBuilder = (slug: string) =>
        `http://127.0.0.1:9100/cockpit/mcp/${slug}`
      const result = await migrateMcpUrls(newBuilder)
      expect(result.migrated).toBe(1)
      expect(result.failed).toBe(1)
      // The healthy profile got its URL rewritten.
      const stored = await readJson(
        `agents/${ok.id}.json`,
        storedAgentProfileSchema,
      )
      expect(stored.mcpUrl).toContain('/cockpit/mcp/')
    })
  })

  test('an empty agents directory returns zero counts and does not throw', async () => {
    await withTempBrowserosDir(async () => {
      const result = await migrateMcpUrls(
        (slug) => `http://127.0.0.1:9100/cockpit/mcp/${slug}`,
      )
      expect(result).toEqual({ migrated: 0, skipped: 0, failed: 0 })
    })
  })
})
