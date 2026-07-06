/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-memory no-op `McpManager` for tests. Real agent-mcp-manager
 * writes to per-user config files (`~/.claude.json`, `~/.cursor/
 * mcp.json`, ...); we never want tests to touch those, so every test
 * that runs through `withTempBrowserClawDir` gets this stub installed
 * by default.
 *
 * Tests that need to assert on install behaviour can grab a fresh
 * stub via `createStubMcpManager()` and inspect its `calls` array.
 */

import type {
  AddServerOptions,
  AddServerResult,
  LinkServerOptions,
  LinkServerResult,
  McpManager,
  RemoveServerOptions,
  UnlinkServerOptions,
  UnlinkServerResult,
} from 'agent-mcp-manager'

export interface StubCall {
  method:
    | 'add'
    | 'link'
    | 'unlink'
    | 'remove'
    | 'listServers'
    | 'listLinks'
    | 'rescan'
  payload: unknown
}

export interface StubMcpManager extends McpManager {
  readonly calls: StubCall[]
  reset(): void
}

export function createStubMcpManager(): StubMcpManager {
  const calls: StubCall[] = []
  return {
    calls,
    reset(): void {
      calls.length = 0
    },
    async add(opts: AddServerOptions): Promise<AddServerResult> {
      calls.push({ method: 'add', payload: opts })
      return { name: opts.name, created: true }
    },
    async link(opts: LinkServerOptions): Promise<LinkServerResult> {
      calls.push({ method: 'link', payload: opts })
      return {
        serverName: opts.serverName,
        agent: opts.agent,
        configPath: opts.configPath ?? `/tmp/stub-${opts.agent}.json`,
        created: true,
      }
    },
    async unlink(opts: UnlinkServerOptions): Promise<UnlinkServerResult> {
      calls.push({ method: 'unlink', payload: opts })
      return {
        serverName: opts.serverName,
        agent: opts.agent,
        configPath: opts.configPath ?? `/tmp/stub-${opts.agent}.json`,
        removed: true,
      }
    },
    async remove(opts: RemoveServerOptions): Promise<void> {
      calls.push({ method: 'remove', payload: opts })
    },
    async listServers() {
      calls.push({ method: 'listServers', payload: {} })
      return []
    },
    async listLinks() {
      calls.push({ method: 'listLinks', payload: {} })
      return []
    },
    async rescan() {
      calls.push({ method: 'rescan', payload: {} })
      return { verified: [], drifted: [], broken: [], unmanaged: [] }
    },
  }
}
