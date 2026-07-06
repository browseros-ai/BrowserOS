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
import type { NewAgentValues } from '../../src/routes/agents/schemas'
import { storedAgentProfileSchema } from '../../src/routes/agents/schemas'
import * as agents from '../../src/routes/agents/service'
import { createStubMcpManager } from '../_helpers/stub-mcp-manager'
import { withTempBrowserClawDir } from '../_helpers/temp-browserclaw-dir'

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
    await withTempBrowserClawDir(async () => {
      const created = await agents.create(makeInput({ name: 'Cowork' }))
      const oldEmbeddedUrl = `http://127.0.0.1:9100/cockpit/mcp/${created.slug}`
      const storedBefore = await readJson(
        `agents/${created.id}.json`,
        storedAgentProfileSchema,
      )
      await writeJson(
        `agents/${created.id}.json`,
        { ...storedBefore, mcpUrl: oldEmbeddedUrl },
        storedAgentProfileSchema,
      )

      const result = await migrateMcpUrls('http://127.0.0.1:9200/mcp')
      expect(result.migrated).toBe(1)
      expect(result.skipped).toBe(0)
      expect(result.failed).toBe(0)

      const stored = await readJson(
        `agents/${created.id}.json`,
        storedAgentProfileSchema,
      )
      expect(stored.mcpUrl).toBe('http://127.0.0.1:9200/mcp')
    })
  })

  test('still handles arbitrary runtime URL changes', async () => {
    await withTempBrowserClawDir(async () => {
      const created = await agents.create(makeInput({ name: 'Other Port' }))
      const result = await migrateMcpUrls('http://127.0.0.1:9100/mcp')
      expect(result.migrated).toBe(1)
      expect(result.skipped).toBe(0)
      expect(result.failed).toBe(0)

      const stored = await readJson(
        `agents/${created.id}.json`,
        storedAgentProfileSchema,
      )
      expect(stored.mcpUrl).toBe('http://127.0.0.1:9100/mcp')
    })
  })

  test('skips a profile whose stored URL already matches the new shape', async () => {
    await withTempBrowserClawDir(async () => {
      const created = await agents.create(makeInput({ name: 'Stable' }))
      const result = await migrateMcpUrls(created.mcpUrl)
      expect(result.migrated).toBe(0)
      expect(result.skipped).toBe(1)
      expect(result.failed).toBe(0)
    })
  })

  test('re-installs the harness entry per migrated row', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      const created = await agents.create(makeInput({ name: 'Reinstall' }))
      stub.reset()
      await migrateMcpUrls('http://127.0.0.1:9100/mcp')
      const methods = stub.calls.map((c) => c.method)
      // Migration: uninstall (old entry) then install (new URL).
      expect(methods).toContain('unlink')
      expect(methods).toContain('add')
      expect(methods).toContain('link')
      const addCall = stub.calls.find((c) => c.method === 'add')
      // `makeInput` defaults to the Claude Desktop harness, whose
      // config parser only validates stdio entries; specFor sources
      // that from the agent-mcp-manager catalog and wraps the URL in
      // `npx mcp-remote`.
      expect(addCall?.payload).toMatchObject({
        name: created.slug,
        spec: {
          transport: 'stdio',
          command: 'npx',
          args: ['mcp-remote', 'http://127.0.0.1:9100/mcp'],
        },
      })
    })
  })

  test('does not advance the stored mcpUrl when harness reinstall fails', async () => {
    await withTempBrowserClawDir(async () => {
      const created = await agents.create(makeInput({ name: 'Retry Install' }))
      const oldUrl = 'http://127.0.0.1:9100/mcp'
      const nextUrl = 'http://127.0.0.1:9200/mcp'
      const storedBefore = await readJson(
        `agents/${created.id}.json`,
        storedAgentProfileSchema,
      )
      await writeJson(
        `agents/${created.id}.json`,
        { ...storedBefore, mcpUrl: oldUrl },
        storedAgentProfileSchema,
      )

      const stub = createStubMcpManager()
      let addAttempts = 0
      stub.add = async () => {
        addAttempts++
        throw new Error('manager add failed')
      }
      setMcpManagerForTesting(stub)

      const first = await migrateMcpUrls(nextUrl)
      expect(first).toEqual({ migrated: 0, skipped: 0, failed: 1 })
      const storedAfterFirst = await readJson(
        `agents/${created.id}.json`,
        storedAgentProfileSchema,
      )
      expect(storedAfterFirst.mcpUrl).toBe(oldUrl)
      expect(addAttempts).toBe(1)

      const second = await migrateMcpUrls(nextUrl)
      expect(second).toEqual({ migrated: 0, skipped: 0, failed: 1 })
      const storedAfterSecond = await readJson(
        `agents/${created.id}.json`,
        storedAgentProfileSchema,
      )
      expect(storedAfterSecond.mcpUrl).toBe(oldUrl)
      expect(addAttempts).toBe(2)
    })
  })

  test('a corrupt profile file is logged + skipped without aborting the sweep', async () => {
    await withTempBrowserClawDir(async (dir) => {
      const ok = await agents.create(makeInput({ name: 'Healthy' }))
      await writeFile(
        join(dir, 'agents', 'broken.json'),
        '{ this is not valid json',
        'utf8',
      )
      const result = await migrateMcpUrls('http://127.0.0.1:9100/mcp')
      expect(result.migrated).toBe(1)
      expect(result.failed).toBe(1)
      // The healthy profile got its URL rewritten.
      const stored = await readJson(
        `agents/${ok.id}.json`,
        storedAgentProfileSchema,
      )
      expect(stored.mcpUrl).toBe('http://127.0.0.1:9100/mcp')
    })
  })

  test('an empty agents directory returns zero counts and does not throw', async () => {
    await withTempBrowserClawDir(async () => {
      const result = await migrateMcpUrls('http://127.0.0.1:9100/mcp')
      expect(result).toEqual({ migrated: 0, skipped: 0, failed: 0 })
    })
  })
})
