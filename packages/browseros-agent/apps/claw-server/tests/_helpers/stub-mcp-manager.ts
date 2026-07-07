/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-memory no-op `BoundApi` for tests. Real agent-mcp-manager writes
 * to per-user config files (`~/.claude.json`, `~/.cursor/mcp.json`,
 * ...); we never want tests to touch those, so every test that runs
 * through `withTempBrowserClawDir` gets this stub installed.
 *
 * Tests that need to assert on install behaviour can grab a fresh
 * stub via `createStubMcpManager()` and inspect its `calls` array.
 * The stub also carries an in-memory manifest so `link` -> `list`
 * roundtrips work for tests that need to observe stored specs.
 */

import type {
  BoundApi,
  ManifestServerEntry,
  McpServer,
} from 'agent-mcp-manager'

export interface StubCall {
  method:
    | 'link'
    | 'unlink'
    | 'disconnect'
    | 'remove'
    | 'list'
    | 'listLinks'
    | 'rescan'
    | 'isInstalled'
  payload: unknown
}

export interface StubMcpManager extends BoundApi {
  readonly calls: StubCall[]
  reset(): void
  /** Direct manipulation of the in-memory manifest for setup shortcuts. */
  seedServer(server: McpServer): void
}

export function createStubMcpManager(): StubMcpManager {
  const calls: StubCall[] = []
  const manifest = new Map<string, ManifestServerEntry>()

  const stub: StubMcpManager = {
    calls,
    reset(): void {
      // Clears the recorded call log so a test can seed manifest
      // state, call reset() to focus on subsequent behaviour, and
      // still exercise the seeded manifest. The manifest is NOT
      // cleared here; callers that need a fully-fresh stub should
      // create a new instance.
      calls.length = 0
    },
    seedServer(server) {
      const now = new Date().toISOString()
      manifest.set(server.name, {
        name: server.name,
        spec: server.spec,
        addedAt: manifest.get(server.name)?.addedAt ?? now,
        links: manifest.get(server.name)?.links ?? {},
      })
    },
    async link(input) {
      calls.push({ method: 'link', payload: input })
      const existing = manifest.get(input.server.name)
      const created = !existing?.links?.[input.agent]
      const now = new Date().toISOString()
      manifest.set(input.server.name, {
        name: input.server.name,
        spec: input.server.spec,
        addedAt: existing?.addedAt ?? now,
        links: {
          ...(existing?.links ?? {}),
          [input.agent]: {
            configPath: input.configPath ?? `/tmp/stub-${input.agent}.json`,
            createdAt: now,
          },
        },
      })
      return {
        serverName: input.server.name,
        agent: input.agent,
        scope: input.scope ?? 'system',
        created,
        overwroteForeign: false,
      }
    },
    async unlink(input) {
      calls.push({ method: 'unlink', payload: input })
      const entry = manifest.get(input.serverName)
      const removed = Boolean(entry?.links?.[input.agent])
      if (entry && removed) {
        const { [input.agent]: _drop, ...rest } = entry.links
        void _drop
        manifest.set(input.serverName, { ...entry, links: rest })
      }
      return {
        serverName: input.serverName,
        agent: input.agent,
        scope: input.scope ?? 'system',
        removed,
      }
    },
    async disconnect(input) {
      calls.push({ method: 'disconnect', payload: input })
      const entry = manifest.get(input.serverName)
      const unlinked = Boolean(entry?.links?.[input.agent])
      let removedManifest = false
      if (entry && unlinked) {
        const { [input.agent]: _drop, ...rest } = entry.links
        void _drop
        const remaining = Object.keys(rest).length
        if (remaining === 0 && input.removeIfLast) {
          manifest.delete(input.serverName)
          removedManifest = true
        } else {
          manifest.set(input.serverName, { ...entry, links: rest })
        }
      }
      return {
        agent: input.agent,
        serverName: input.serverName,
        scope: input.scope ?? 'system',
        unlinked,
        removedManifest,
      }
    },
    async remove(input) {
      calls.push({ method: 'remove', payload: input })
      const entry = manifest.get(input.serverName)
      const unlinkedAgents = entry ? Object.keys(entry.links) : []
      const removedManifest = manifest.delete(input.serverName)
      return {
        serverName: input.serverName,
        unlinkedAgents: unlinkedAgents as never,
        removedManifest,
      }
    },
    async list() {
      calls.push({ method: 'list', payload: {} })
      return Array.from(manifest.values())
    },
    async listLinks(input) {
      calls.push({ method: 'listLinks', payload: input ?? {} })
      const nameFilter = input?.serverNames
      const agentFilter = input?.agents
      const out: Array<{
        serverName: string
        agent: string
        configPath: string
      }> = []
      for (const entry of manifest.values()) {
        if (nameFilter && !nameFilter.includes(entry.name)) continue
        for (const [agent, link] of Object.entries(entry.links)) {
          if (agentFilter && !agentFilter.includes(agent as never)) continue
          if (!link) continue
          out.push({
            serverName: entry.name,
            agent,
            configPath: link.configPath,
          })
        }
      }
      return out as never
    },
    async rescan(input) {
      calls.push({ method: 'rescan', payload: input ?? {} })
      return { verified: [], drifted: [], missing: [] }
    },
    async isInstalled(input) {
      calls.push({ method: 'isInstalled', payload: input })
      const out: Partial<Record<string, boolean>> = {}
      for (const agent of input.agents) out[agent] = true
      return out as never
    },
  }
  return stub
}
