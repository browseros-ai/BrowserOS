/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-memory `BoundApi` for tests. The real `@browseros/agent-mcp-
 * manager` writes to per-user config files (`~/.claude.json`,
 * `~/.cursor/mcp.json`, ...); unit tests never want to touch those,
 * so they install this stub via `setMcpManagerForTesting`.
 *
 * Carries an in-memory manifest so `link` -> `list`/`listLinks`
 * roundtrips work, records every verb call in `calls`, and exposes
 * throw hooks so a test can force a `ForeignEntryError`, a
 * `ServerNotFoundError`, or a per-agent link failure.
 */

import type {
  BoundApi,
  ManifestServerEntry,
  McpServerSpec,
} from '@browseros/agent-mcp-manager'

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

export interface StubOptions {
  /** Throw this error from `disconnect`/`unlink` for the given server name. */
  removeThrowsByServer?: Map<string, Error>
  /** Throw an error from `link` for these agent ids. */
  linkThrowsByAgent?: Set<string>
  /**
   * Agent ids `rescan` reports as drifted (manifest link exists but the
   * on-disk entry is gone) rather than verified. Models a user who hand
   * removed the entry.
   */
  rescanDriftedAgents?: Set<string>
  /** Agent ids `rescan` reports as missing (config file gone). */
  rescanMissingAgents?: Set<string>
}

export interface SeedLink {
  agent: string
  configPath?: string
}

export interface StubMcpManager extends BoundApi {
  readonly calls: StubCall[]
  seedServer(name: string, spec: McpServerSpec, links?: SeedLink[]): void
}

export function createStubMcpManager(
  options: StubOptions = {},
): StubMcpManager {
  const calls: StubCall[] = []
  const manifest = new Map<string, ManifestServerEntry>()
  const removeThrows = options.removeThrowsByServer ?? new Map<string, Error>()
  const linkThrows = options.linkThrowsByAgent ?? new Set<string>()
  const rescanDrifted = options.rescanDriftedAgents ?? new Set<string>()
  const rescanMissing = options.rescanMissingAgents ?? new Set<string>()

  const stub: StubMcpManager = {
    calls,
    seedServer(name, spec, links = []) {
      const now = '2026-06-11T00:00:00.000Z'
      manifest.set(name, {
        name,
        spec,
        addedAt: now,
        links: Object.fromEntries(
          links.map((l) => [
            l.agent,
            {
              configPath: l.configPath ?? `/tmp/stub-${l.agent}.json`,
              createdAt: now,
            },
          ]),
        ),
      })
    },
    async link(input) {
      calls.push({ method: 'link', payload: input })
      if (linkThrows.has(input.agent)) {
        throw new Error(`Permission denied for ${input.agent}`)
      }
      const existing = manifest.get(input.server.name)
      const created = !existing?.links?.[input.agent]
      const now = '2026-06-11T00:00:00.000Z'
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
      const forced = removeThrows.get(input.serverName)
      if (forced) throw forced
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
      const forced = removeThrows.get(input.serverName)
      if (forced) throw forced
      const entry = manifest.get(input.serverName)
      const unlinked = Boolean(entry?.links?.[input.agent])
      let removedManifest = false
      if (entry && unlinked) {
        const { [input.agent]: _drop, ...rest } = entry.links
        void _drop
        if (Object.keys(rest).length === 0 && input.removeIfLast) {
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
      const agentFilter = input?.agents
      const verified: Array<{
        serverName: string
        agent: string
        configPath: string
      }> = []
      const drifted: Array<{
        serverName: string
        agent: string
        scope: string
        configPath: string
        reason: string
      }> = []
      const missing: typeof drifted = []
      for (const entry of manifest.values()) {
        for (const [agent, link] of Object.entries(entry.links)) {
          if (!link) continue
          if (agentFilter && !agentFilter.includes(agent as never)) continue
          const base = {
            serverName: entry.name,
            agent,
            scope: 'system',
            configPath: link.configPath,
          }
          if (rescanMissing.has(agent)) {
            missing.push({ ...base, reason: 'config file does not exist' })
          } else if (rescanDrifted.has(agent)) {
            drifted.push({ ...base, reason: 'no matching entry on disk' })
          } else {
            verified.push({
              serverName: entry.name,
              agent,
              configPath: link.configPath,
            })
          }
        }
      }
      return { verified, drifted, missing } as never
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

/** Convenience: filter the recorded calls to one verb. */
export function callsOf(
  stub: StubMcpManager,
  method: StubCall['method'],
): unknown[] {
  return stub.calls.filter((c) => c.method === method).map((c) => c.payload)
}
