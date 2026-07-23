/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Boot-time config-path migration for the shared BrowserClaw MCP
 * server. When the agent catalog's OS-resolved default path moves
 * between BrowserClaw versions (e.g. Antigravity 1.x -> 2.x moved
 * from `~/.gemini/antigravity/` to `~/.gemini/config/`), an existing
 * install's manifest still records the OLD path and every downstream
 * self-heal loop keeps writing there. Users see a green "Configured"
 * badge while the harness reads a different file and never sees the
 * entry.
 *
 * This loop diffs the manifest-recorded configPath against the
 * catalog's current default and, when they disagree, relocates the
 * entry: strip from OLD, write at NEW, update the link record. Per
 * link failures are isolated. A durable pending marker forces retry
 * on the next boot when a run is interrupted.
 */

import { access, unlink, writeFile } from 'node:fs/promises'
import { homedir } from 'node:os'
import { join, resolve as resolvePath } from 'node:path'
import type {
  AgentId,
  AgentScope,
  BoundApi,
  ManifestServerEntry,
  McpServerSpec,
} from '@browseros/agent-mcp-manager'
import { resolveAgentMcpConfigPath as defaultResolveAgentMcpConfigPath } from '@browseros/agent-mcp-manager'
import { BROWSEROS_MCP_SERVER_NAME } from '../shared/mcp-url'
import { resolveClawServerPath } from './browserclaw-dir'
import { logger } from './logger'
import { getMcpManager } from './mcp-manager'

const MIGRATION_PENDING_FILE = 'mcp-config-path-migration.pending'

interface MigrationCounters {
  migrated: number
  skipped: number
  failed: number
}

type ConfigPathResolver = (agent: AgentId, scope: AgentScope) => Promise<string>

let configPathResolver: ConfigPathResolver = (agent, scope) =>
  defaultResolveAgentMcpConfigPath(agent, scope)

/**
 * Test hook. Swaps the OS-default resolver so unit tests can stage
 * arbitrary "current default" paths without shelling out to the
 * per-OS filesystem probing inside `resolveAgentMcpConfigPath`.
 */
export function setConfigPathResolverForTesting(
  next: ConfigPathResolver | null,
): void {
  configPathResolver =
    next ?? ((agent, scope) => defaultResolveAgentMcpConfigPath(agent, scope))
}

export async function migrateMcpConfigPaths(): Promise<MigrationCounters> {
  const mgr = getMcpManager()
  const counters: MigrationCounters = { migrated: 0, skipped: 0, failed: 0 }
  const servers = await safeList(mgr)
  if (servers === null) return counters

  const server = servers.find(
    (entry) => entry.name === BROWSEROS_MCP_SERVER_NAME,
  )
  if (!server) return counters

  const hasPendingMigration = await pendingMigrationExists()
  const targets = await collectTargets(server, counters)
  if (targets.length === 0) {
    if (hasPendingMigration && !(await clearPendingMarker())) {
      counters.failed++
    }
    return counters
  }
  if (!(await writePendingMarker())) {
    counters.failed++
    return counters
  }

  for (const target of targets) {
    const ok = await migrateOne(mgr, server, target)
    if (ok) counters.migrated++
    else counters.failed++
  }
  if (counters.failed === 0 && !(await clearPendingMarker())) {
    counters.failed++
  }
  return counters
}

interface MigrationTarget {
  agent: AgentId
  scope: AgentScope
  oldConfigPath: string
  newConfigPath: string
}

async function collectTargets(
  server: ManifestServerEntry,
  counters: MigrationCounters,
): Promise<MigrationTarget[]> {
  const targets: MigrationTarget[] = []
  for (const agentRaw of Object.keys(server.links)) {
    const agent = agentRaw as AgentId
    const link = server.links[agent]
    if (!link) continue
    const scope: AgentScope = 'system'
    let resolvedDefault: string
    try {
      resolvedDefault = await configPathResolver(agent, scope)
    } catch (err) {
      counters.skipped++
      logger.info('mcpConfigPath migration: default path unresolved, skipped', {
        serverName: BROWSEROS_MCP_SERVER_NAME,
        agent,
        error: err instanceof Error ? err.message : String(err),
      })
      continue
    }
    if (samePath(link.configPath, resolvedDefault)) {
      counters.skipped++
      continue
    }
    targets.push({
      agent,
      scope,
      oldConfigPath: link.configPath,
      newConfigPath: resolvedDefault,
    })
  }
  return targets
}

async function migrateOne(
  mgr: BoundApi,
  server: ManifestServerEntry,
  target: MigrationTarget,
): Promise<boolean> {
  const spec = server.spec
  try {
    await mgr.unlink({
      serverName: BROWSEROS_MCP_SERVER_NAME,
      agent: target.agent,
      scope: target.scope,
      configPath: target.oldConfigPath,
    })
  } catch (err) {
    logger.warn('mcpConfigPath migration: unlink at old path failed', {
      serverName: BROWSEROS_MCP_SERVER_NAME,
      agent: target.agent,
      oldConfigPath: target.oldConfigPath,
      error: err instanceof Error ? err.message : String(err),
    })
    return false
  }
  try {
    await mgr.link({
      server: { name: BROWSEROS_MCP_SERVER_NAME, spec },
      agent: target.agent,
      scope: target.scope,
      allowOverwrite: true,
    })
    logger.info('mcpConfigPath migration: relocated', {
      serverName: BROWSEROS_MCP_SERVER_NAME,
      agent: target.agent,
      from: target.oldConfigPath,
      to: target.newConfigPath,
    })
    return true
  } catch (err) {
    logger.warn('mcpConfigPath migration: link at new path failed', {
      serverName: BROWSEROS_MCP_SERVER_NAME,
      agent: target.agent,
      newConfigPath: target.newConfigPath,
      error: err instanceof Error ? err.message : String(err),
    })
    await tryRestore(mgr, target, spec)
    return false
  }
}

async function tryRestore(
  mgr: BoundApi,
  target: MigrationTarget,
  spec: McpServerSpec,
): Promise<void> {
  try {
    await mgr.link({
      server: { name: BROWSEROS_MCP_SERVER_NAME, spec },
      agent: target.agent,
      scope: target.scope,
      configPath: target.oldConfigPath,
      allowOverwrite: true,
    })
    logger.info('mcpConfigPath migration: restored link at old path', {
      serverName: BROWSEROS_MCP_SERVER_NAME,
      agent: target.agent,
      oldConfigPath: target.oldConfigPath,
    })
  } catch (err) {
    logger.error('mcpConfigPath migration: restore failed', {
      serverName: BROWSEROS_MCP_SERVER_NAME,
      agent: target.agent,
      oldConfigPath: target.oldConfigPath,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}

function samePath(a: string, b: string): boolean {
  return resolvePath(expandHome(a)) === resolvePath(expandHome(b))
}

// `path.resolve` does NOT expand `~`. The library's own resolver
// always emits `$HOME`-expanded absolute paths into the manifest,
// so this branch is defensive: it prevents a boot-time re-migration
// loop if a direct API caller (or a future manifest-format change)
// hands us a `~`-prefixed configPath.
function expandHome(p: string): string {
  if (p === '~') return homedir()
  if (p.startsWith('~/') || p.startsWith('~\\')) {
    return join(homedir(), p.slice(2))
  }
  return p
}

async function safeList(
  mgr: BoundApi,
): Promise<Awaited<ReturnType<BoundApi['list']>> | null> {
  try {
    return await mgr.list()
  } catch (err) {
    logger.warn('mcpConfigPath migration: manifest list failed', {
      error: err instanceof Error ? err.message : String(err),
    })
    return null
  }
}

async function pendingMigrationExists(): Promise<boolean> {
  try {
    await access(resolveClawServerPath(MIGRATION_PENDING_FILE))
    return true
  } catch (err) {
    if (isFsError(err, 'ENOENT')) return false
    logger.warn('mcpConfigPath migration: pending marker check failed', {
      error: err instanceof Error ? err.message : String(err),
    })
    return true
  }
}

async function writePendingMarker(): Promise<boolean> {
  try {
    await writeFile(
      resolveClawServerPath(MIGRATION_PENDING_FILE),
      `${new Date().toISOString()}\n`,
      { encoding: 'utf8', mode: 0o600 },
    )
    return true
  } catch (err) {
    logger.warn('mcpConfigPath migration: could not write pending marker', {
      error: err instanceof Error ? err.message : String(err),
    })
    return false
  }
}

async function clearPendingMarker(): Promise<boolean> {
  try {
    await unlink(resolveClawServerPath(MIGRATION_PENDING_FILE))
    return true
  } catch (err) {
    if (isFsError(err, 'ENOENT')) return true
    logger.warn('mcpConfigPath migration: could not clear pending marker', {
      error: err instanceof Error ? err.message : String(err),
    })
    return false
  }
}

function isFsError(err: unknown, code: string): boolean {
  return (
    typeof err === 'object' &&
    err !== null &&
    'code' in err &&
    (err as { code?: unknown }).code === code
  )
}
