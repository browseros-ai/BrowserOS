/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Boot-time URL migration for every managed MCP server.
 *
 * Walks the workspace manifest via `list()` and, for each server
 * whose spec URL differs from the runtime's current canonical MCP
 * URL, re-links every agent that was previously linked to it with a
 * fresh spec pointing at the new URL. Also rewrites any matching
 * stored profile JSON so the cockpit's mcpUrl field stays in sync
 * with what actually landed on disk.
 *
 * Covers BOTH per-profile installs AND the shared `BrowserClaw`
 * server entry written by the Connect button; the pre-0.0.4 version
 * of this sweep only touched profile JSON.
 *
 * Failures log per entry; a single bad link does not abort the sweep.
 * The migration is idempotent: a second run against the same
 * `targetMcpUrl` is a no-op once every spec URL has been refreshed.
 */

import type { McpServerSpec } from 'agent-mcp-manager'
import {
  type StoredAgentProfile,
  storedAgentProfileSchema,
} from '../routes/agents/schemas'
import { logger } from './logger'
import { getMcpManager } from './mcp-manager'
import { listFiles, readJson, writeJson } from './storage'

const AGENTS_SUBDIR = 'agents'

export async function migrateMcpUrls(
  targetMcpUrl: string,
): Promise<{ migrated: number; skipped: number; failed: number }> {
  const mgr = getMcpManager()
  let migrated = 0
  let skipped = 0
  let failed = 0

  let servers: Awaited<ReturnType<typeof mgr.list>>
  try {
    servers = await mgr.list()
  } catch (err) {
    logger.warn('mcpUrl migration: manifest list failed', {
      error: err instanceof Error ? err.message : String(err),
    })
    return { migrated, skipped, failed }
  }

  for (const server of servers) {
    const currentUrl = extractSpecUrl(server.spec)
    if (currentUrl === null || currentUrl === targetMcpUrl) {
      skipped++
      continue
    }
    const nextSpec = rewriteSpecUrl(server.spec, targetMcpUrl)
    const linkedAgents = Object.keys(server.links) as Array<
      keyof typeof server.links
    >
    for (const agent of linkedAgents) {
      try {
        await mgr.link({
          server: { name: server.name, spec: nextSpec },
          agent,
          allowOverwrite: true,
        })
        migrated++
        logger.info('mcpUrl migration: relinked', {
          serverName: server.name,
          agent,
          from: currentUrl,
          to: targetMcpUrl,
        })
      } catch (err) {
        failed++
        logger.warn('mcpUrl migration: relink failed', {
          serverName: server.name,
          agent,
          error: err instanceof Error ? err.message : String(err),
        })
      }
    }
  }

  // Rewrite any stored profile JSON files whose cached mcpUrl no
  // longer matches. Profiles are matched to manifest entries by slug
  // (profile.slug === server.name for profile installs).
  const profileNames = await listFiles(AGENTS_SUBDIR)
  for (const name of profileNames) {
    const file = `${AGENTS_SUBDIR}/${name}`
    try {
      const profile = await readJson(file, storedAgentProfileSchema)
      if (profile.mcpUrl === targetMcpUrl) continue
      const updated: StoredAgentProfile = { ...profile, mcpUrl: targetMcpUrl }
      await writeJson(file, updated, storedAgentProfileSchema)
      logger.info('mcpUrl migration: updated stored profile', {
        slug: profile.slug,
        from: profile.mcpUrl,
        to: targetMcpUrl,
      })
    } catch (err) {
      failed++
      logger.warn('mcpUrl migration: profile rewrite failed', {
        file,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  return { migrated, skipped, failed }
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
  return {
    ...spec,
    args: (spec.args ?? []).map((a) => (/^https?:\/\//.test(a) ? newUrl : a)),
  }
}
