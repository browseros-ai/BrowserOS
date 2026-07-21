/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Boot-time URL migration for the shared BrowserClaw MCP server.
 *
 * When the proxy or bind port changes between runs, the canonical MCP
 * URL moves. Walk the manager manifest, find the shared `BrowserClaw`
 * entry written by the live connection routes, and re-link each harness
 * with a fresh spec. Other managed entries are outside this runtime's
 * current connection model and are deliberately ignored.
 *
 * Failures are isolated per harness so one unwritable config does not
 * prevent the remaining links from advancing. A durable pending marker
 * is written before the first relink and cleared only after all links
 * succeed, so a crash or partial failure is retried on the next boot.
 */

import { access, unlink, writeFile } from 'node:fs/promises'
import type {
  AgentId,
  BoundApi,
  McpServerSpec,
} from '@browseros/agent-mcp-manager'
import { BROWSEROS_MCP_SERVER_NAME } from '../shared/mcp-url'
import { resolveClawServerPath } from './browserclaw-dir'
import { logger } from './logger'
import { getMcpManager } from './mcp-manager'

const MIGRATION_PENDING_FILE = 'mcp-url-migration.pending'

interface MigrationCounters {
  migrated: number
  skipped: number
  failed: number
}

export async function migrateMcpUrls(
  targetMcpUrl: string,
): Promise<MigrationCounters> {
  const mgr = getMcpManager()
  const counters: MigrationCounters = { migrated: 0, skipped: 0, failed: 0 }
  const servers = await safeList(mgr)
  if (servers === null) return counters

  const server = servers.find(
    (entry) => entry.name === BROWSEROS_MCP_SERVER_NAME,
  )
  if (!server) return counters

  const hasPendingMigration = await pendingMigrationExists()
  const currentUrl = extractSpecUrl(server.spec)
  if (
    currentUrl === null ||
    (currentUrl === targetMcpUrl && !hasPendingMigration)
  ) {
    counters.skipped++
    return counters
  }
  if (!(await writePendingMarker(targetMcpUrl))) {
    counters.failed++
    return counters
  }

  const nextSpec = rewriteSpecUrl(server.spec, targetMcpUrl)
  for (const agent of Object.keys(server.links) as AgentId[]) {
    const configPath = server.links[agent]?.configPath
    if (!configPath) {
      counters.failed++
      logger.warn('mcpUrl migration: link has no recorded config path', {
        serverName: BROWSEROS_MCP_SERVER_NAME,
        agent,
      })
      continue
    }
    const ok = await relinkOne(
      mgr,
      nextSpec,
      agent,
      configPath,
      currentUrl,
      targetMcpUrl,
    )
    if (ok) counters.migrated++
    else counters.failed++
  }
  if (counters.failed === 0 && !(await clearPendingMarker())) {
    counters.failed++
  }
  return counters
}

async function safeList(
  mgr: BoundApi,
): Promise<Awaited<ReturnType<BoundApi['list']>> | null> {
  try {
    return await mgr.list()
  } catch (err) {
    logger.warn('mcpUrl migration: manifest list failed', {
      error: err instanceof Error ? err.message : String(err),
    })
    return null
  }
}

async function relinkOne(
  mgr: BoundApi,
  nextSpec: McpServerSpec,
  agent: AgentId,
  configPath: string,
  fromUrl: string,
  toUrl: string,
): Promise<boolean> {
  try {
    await mgr.link({
      server: { name: BROWSEROS_MCP_SERVER_NAME, spec: nextSpec },
      agent,
      configPath,
      allowOverwrite: true,
    })
    logger.info('mcpUrl migration: relinked', {
      serverName: BROWSEROS_MCP_SERVER_NAME,
      agent,
      from: fromUrl,
      to: toUrl,
    })
    return true
  } catch (err) {
    logger.warn('mcpUrl migration: relink failed', {
      serverName: BROWSEROS_MCP_SERVER_NAME,
      agent,
      error: err instanceof Error ? err.message : String(err),
    })
    return false
  }
}

function extractSpecUrl(spec: McpServerSpec): string | null {
  if (spec.transport === 'http' || spec.transport === 'sse') return spec.url
  if (spec.transport === 'stdio') {
    const urlArg = spec.args?.find((a) => /^https?:\/\//.test(a))
    return urlArg ?? null
  }
  return null
}

function rewriteSpecUrl(spec: McpServerSpec, newUrl: string): McpServerSpec {
  if (spec.transport === 'http' || spec.transport === 'sse') {
    return { ...spec, url: newUrl }
  }
  // Rewrite only the first HTTP-like arg to match `extractSpecUrl`'s
  // `Array.find` semantics. A later HTTP arg may serve another purpose.
  const args = spec.args ?? []
  const firstUrlIdx = args.findIndex((a) => /^https?:\/\//.test(a))
  if (firstUrlIdx === -1) return { ...spec }
  const nextArgs = args.slice()
  nextArgs[firstUrlIdx] = newUrl
  return { ...spec, args: nextArgs }
}

async function pendingMigrationExists(): Promise<boolean> {
  try {
    await access(resolveClawServerPath(MIGRATION_PENDING_FILE))
    return true
  } catch (err) {
    if (isFsError(err, 'ENOENT')) return false
    logger.warn('mcpUrl migration: pending marker check failed', {
      error: err instanceof Error ? err.message : String(err),
    })
    return true
  }
}

async function writePendingMarker(targetMcpUrl: string): Promise<boolean> {
  try {
    await writeFile(
      resolveClawServerPath(MIGRATION_PENDING_FILE),
      `${targetMcpUrl}\n`,
      { encoding: 'utf8', mode: 0o600 },
    )
    return true
  } catch (err) {
    logger.warn('mcpUrl migration: could not write pending marker', {
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
    logger.warn('mcpUrl migration: could not clear pending marker', {
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
