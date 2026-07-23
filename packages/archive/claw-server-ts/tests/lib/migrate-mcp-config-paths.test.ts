/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, describe, expect, test } from 'bun:test'
import { stat, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join } from 'node:path'
import { setMcpManagerForTesting } from '../../src/lib/mcp-manager'
import {
  migrateMcpConfigPaths,
  setConfigPathResolverForTesting,
} from '../../src/lib/migrate-mcp-config-paths'
import { createStubMcpManager } from '../_helpers/stub-mcp-manager'
import { withTempBrowserClawDir } from '../_helpers/temp-browserclaw-dir'

const OLD_ANTI = '/tmp/legacy/.gemini/antigravity/mcp_config.json'
// Stub records `/tmp/stub-${agent}.json` when `mgr.link` is called
// without a configPath override. Tests align "current OS default"
// with that so a migration relinks to what the stub would emit.
const NEW_ANTI = '/tmp/stub-antigravity.json'
const CLAUDE_HOME = '/tmp/stub-claude-code.json'
const CURSOR_HOME = '/tmp/stub-cursor.json'
const URL = 'http://127.0.0.1:9200/mcp'

afterEach(() => {
  setConfigPathResolverForTesting(null)
})

describe('migrateMcpConfigPaths', () => {
  test('an empty manifest returns zero counts and does not throw', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      setMcpManagerForTesting(stub)
      setConfigPathResolverForTesting(async () => NEW_ANTI)
      const result = await migrateMcpConfigPaths()
      expect(result).toEqual({ migrated: 0, skipped: 0, failed: 0 })
    })
  })

  test('no BrowserClaw server in the manifest is a no-op', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      await stub.link({
        server: {
          name: 'some-other-server',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        configPath: OLD_ANTI,
      })
      setMcpManagerForTesting(stub)
      setConfigPathResolverForTesting(async () => NEW_ANTI)
      stub.reset()
      const result = await migrateMcpConfigPaths()
      expect(result).toEqual({ migrated: 0, skipped: 0, failed: 0 })
      expect(stub.calls.some((c) => c.method === 'link')).toBe(false)
      expect(stub.calls.some((c) => c.method === 'unlink')).toBe(false)
    })
  })

  test('a link whose configPath already matches the OS default is skipped', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        configPath: NEW_ANTI,
      })
      setMcpManagerForTesting(stub)
      setConfigPathResolverForTesting(async () => NEW_ANTI)
      stub.reset()
      const result = await migrateMcpConfigPaths()
      expect(result).toEqual({ migrated: 0, skipped: 1, failed: 0 })
      expect(stub.calls.some((c) => c.method === 'link')).toBe(false)
      expect(stub.calls.some((c) => c.method === 'unlink')).toBe(false)
    })
  })

  test('a tilde-prefixed manifest path is treated as equal to its expanded home form', async () => {
    // Defensive against a direct API caller (or future manifest
    // format change) that hands us `~/...`. The library today
    // always emits `$HOME`-expanded absolute paths, so this branch
    // is not reachable via the manager's own writes, but the
    // migration is idempotent enough that a false-positive here
    // would silently loop every boot.
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      const tildePath = '~/.gemini/config/mcp_config.json'
      const expanded = join(homedir(), '.gemini/config/mcp_config.json')
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        configPath: tildePath,
      })
      setMcpManagerForTesting(stub)
      setConfigPathResolverForTesting(async () => expanded)
      stub.reset()

      const result = await migrateMcpConfigPaths()

      expect(result).toEqual({ migrated: 0, skipped: 1, failed: 0 })
      expect(stub.calls.some((c) => c.method === 'link')).toBe(false)
      expect(stub.calls.some((c) => c.method === 'unlink')).toBe(false)
    })
  })

  test('a link whose configPath differs is unlinked at OLD and relinked at NEW', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        configPath: OLD_ANTI,
      })
      setMcpManagerForTesting(stub)
      setConfigPathResolverForTesting(async () => NEW_ANTI)
      stub.reset()

      const result = await migrateMcpConfigPaths()

      expect(result).toEqual({ migrated: 1, skipped: 0, failed: 0 })
      const unlinkCall = stub.calls.find((c) => c.method === 'unlink')
      expect(unlinkCall?.payload).toMatchObject({
        serverName: 'BrowserClaw',
        agent: 'antigravity',
        configPath: OLD_ANTI,
      })
      const linkCall = stub.calls.find((c) => c.method === 'link')
      expect(linkCall?.payload).toMatchObject({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        allowOverwrite: true,
      })
      expect(
        (linkCall?.payload as { configPath?: string }).configPath,
      ).toBeUndefined()
      const [server] = await stub.list()
      expect(server?.links.antigravity?.configPath).toBe(NEW_ANTI)
    })
  })

  test('multiple links: matching ones skipped, mismatching ones migrated', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        configPath: OLD_ANTI,
      })
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'claude-code',
        configPath: CLAUDE_HOME,
      })
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'cursor',
        configPath: CURSOR_HOME,
      })
      setMcpManagerForTesting(stub)
      setConfigPathResolverForTesting(async (agent) => {
        if (agent === 'antigravity') return NEW_ANTI
        if (agent === 'claude-code') return CLAUDE_HOME
        if (agent === 'cursor') return CURSOR_HOME
        throw new Error(`unexpected agent ${agent}`)
      })
      stub.reset()

      const result = await migrateMcpConfigPaths()

      expect(result).toEqual({ migrated: 1, skipped: 2, failed: 0 })
      expect(stub.calls.filter((c) => c.method === 'unlink')).toHaveLength(1)
      expect(stub.calls.filter((c) => c.method === 'link')).toHaveLength(1)
    })
  })

  test('resolver throw for an agent skips that link and keeps sweeping others', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        configPath: OLD_ANTI,
      })
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'claude-code',
        configPath: '/tmp/old-claude.json',
      })
      setMcpManagerForTesting(stub)
      setConfigPathResolverForTesting(async (agent) => {
        if (agent === 'antigravity') {
          throw new Error('antigravity no longer installed')
        }
        return CLAUDE_HOME
      })
      stub.reset()

      const result = await migrateMcpConfigPaths()

      expect(result).toEqual({ migrated: 1, skipped: 1, failed: 0 })
      const linkCall = stub.calls.find(
        (c) =>
          c.method === 'link' &&
          (c.payload as { agent?: string }).agent === 'claude-code',
      )
      expect(linkCall).toBeDefined()
    })
  })

  test('link failure after unlink success attempts restore and counts as failed', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        configPath: OLD_ANTI,
      })
      const originalLink = stub.link
      let sawOverride = false
      stub.link = async (input) => {
        if (input.configPath === undefined) {
          throw new Error('new-path write locked')
        }
        if (input.configPath === OLD_ANTI) sawOverride = true
        return originalLink(input)
      }
      setMcpManagerForTesting(stub)
      setConfigPathResolverForTesting(async () => NEW_ANTI)
      stub.reset()

      const result = await migrateMcpConfigPaths()

      expect(result).toEqual({ migrated: 0, skipped: 0, failed: 1 })
      expect(sawOverride).toBe(true)
      const [server] = await stub.list()
      expect(server?.links.antigravity?.configPath).toBe(OLD_ANTI)
    })
  })

  test('non-BrowserClaw manifest entries are ignored', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        configPath: OLD_ANTI,
      })
      await stub.link({
        server: {
          name: 'legacy-profile-slug',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        configPath: '/tmp/some-other-file.json',
      })
      setMcpManagerForTesting(stub)
      setConfigPathResolverForTesting(async () => NEW_ANTI)
      stub.reset()

      const result = await migrateMcpConfigPaths()

      expect(result).toEqual({ migrated: 1, skipped: 0, failed: 0 })
      const unlinkCalls = stub.calls.filter((c) => c.method === 'unlink')
      expect(unlinkCalls).toHaveLength(1)
      expect(unlinkCalls[0]?.payload).toMatchObject({
        serverName: 'BrowserClaw',
      })
    })
  })

  test('keeps a pending marker so an interrupted run retries on next boot', async () => {
    await withTempBrowserClawDir(async () => {
      const stub = createStubMcpManager()
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        configPath: OLD_ANTI,
      })
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'claude-code',
        configPath: '/tmp/old-claude.json',
      })
      const originalLink = stub.link
      let failAnti = true
      stub.link = async (input) => {
        if (
          failAnti &&
          input.agent === 'antigravity' &&
          input.configPath === undefined
        ) {
          throw new Error('anti locked')
        }
        return originalLink(input)
      }
      setMcpManagerForTesting(stub)
      setConfigPathResolverForTesting(async (agent) =>
        agent === 'antigravity' ? NEW_ANTI : CLAUDE_HOME,
      )
      stub.reset()

      const first = await migrateMcpConfigPaths()

      expect(first).toEqual({ migrated: 1, skipped: 0, failed: 1 })

      failAnti = false
      stub.reset()
      const second = await migrateMcpConfigPaths()

      expect(second).toEqual({ migrated: 1, skipped: 1, failed: 0 })
      const [server] = await stub.list()
      expect(server?.links.antigravity?.configPath).toBe(NEW_ANTI)
      expect(server?.links['claude-code']?.configPath).toBe(CLAUDE_HOME)
    })
  })

  test('a stale pending marker with nothing to migrate is cleared', async () => {
    await withTempBrowserClawDir(async (root) => {
      const marker = join(root, 'mcp-config-path-migration.pending')
      await writeFile(marker, 'stale', 'utf8')
      const stub = createStubMcpManager()
      await stub.link({
        server: {
          name: 'BrowserClaw',
          spec: { transport: 'http', url: URL },
        },
        agent: 'antigravity',
        configPath: NEW_ANTI,
      })
      setMcpManagerForTesting(stub)
      setConfigPathResolverForTesting(async () => NEW_ANTI)
      stub.reset()

      const result = await migrateMcpConfigPaths()

      expect(result).toEqual({ migrated: 0, skipped: 1, failed: 0 })
      const stillExists = await stat(marker)
        .then(() => true)
        .catch(() => false)
      expect(stillExists).toBe(false)
    })
  })
})
