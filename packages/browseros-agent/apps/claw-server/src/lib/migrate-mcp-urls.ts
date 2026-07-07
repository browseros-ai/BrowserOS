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
 * fresh spec pointing at the new URL. Also rewrites stored profile
 * JSON so the cockpit's `mcpUrl` field stays in sync with what
 * actually landed on disk, BUT only for profiles whose corresponding
 * manifest relink actually succeeded; a failed link must leave the
 * profile JSON on the old URL so the profile / manifest / harness
 * config never enter a three-way split state across reboots.
 *
 * Covers BOTH per-profile installs AND the shared `BrowserClaw`
 * server entry written by the Connect button; the pre-0.0.4 version
 * of this sweep only touched profile JSON.
 *
 * Failures log per entry; a single bad link does not abort the sweep.
 * Retired-harness profile files (created before a harness was dropped
 * from `harnessEnum`) are quarantined into an `agents/.retired/`
 * subdir so `readJson` does not throw a Zod parse error on every
 * subsequent boot. The migration is idempotent: a second run against
 * the same `targetMcpUrl` is a no-op once every spec URL has been
 * refreshed.
 */

import { rename } from 'node:fs/promises'
import type { BoundApi, McpServerSpec } from 'agent-mcp-manager'
import {
  type StoredAgentProfile,
  storedAgentProfileSchema,
} from '../routes/agents/schemas'
import { resolveClawServerPath } from './browserclaw-dir'
import { logger } from './logger'
import { getMcpManager } from './mcp-manager'
import { ensureDir, listFiles, readJsonRaw, writeJson } from './storage'

const AGENTS_SUBDIR = 'agents'
const RETIRED_SUBDIR = 'agents/.retired'

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

  const migratedServerNames = await relinkManifest(
    mgr,
    servers,
    targetMcpUrl,
    counters,
  )
  await rewriteProfiles(servers, migratedServerNames, targetMcpUrl, counters)
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

async function relinkManifest(
  mgr: BoundApi,
  servers: Awaited<ReturnType<BoundApi['list']>>,
  targetMcpUrl: string,
  counters: MigrationCounters,
): Promise<Set<string>> {
  // Track which server names' manifest relink actually succeeded, so
  // the profile JSON rewrite below can skip profiles whose manifest
  // entry never advanced. Without this gate, a failed mgr.link leaves
  // the harness config on the old URL while the profile JSON says the
  // new URL, producing a three-way split (profile / manifest / disk)
  // that survives across reboots.
  const migratedServerNames = new Set<string>()

  for (const server of servers) {
    const currentUrl = extractSpecUrl(server.spec)
    if (currentUrl === null || currentUrl === targetMcpUrl) {
      counters.skipped++
      continue
    }
    const nextSpec = rewriteSpecUrl(server.spec, targetMcpUrl)
    const linkedAgents = Object.keys(server.links) as Array<
      keyof typeof server.links
    >
    let anyFailure = false
    for (const agent of linkedAgents) {
      const ok = await relinkOne(
        mgr,
        server.name,
        nextSpec,
        agent,
        currentUrl,
        targetMcpUrl,
      )
      if (ok) counters.migrated++
      else {
        counters.failed++
        anyFailure = true
      }
    }
    // Only mark the server as migrated when EVERY agent's link
    // succeeded. A partial failure leaves the manifest entry with
    // mixed on-disk state; withholding the profile JSON write means
    // the next boot retries the same set until they all land.
    if (linkedAgents.length > 0 && !anyFailure) {
      migratedServerNames.add(server.name)
    }
  }

  return migratedServerNames
}

async function relinkOne(
  mgr: BoundApi,
  serverName: string,
  nextSpec: McpServerSpec,
  agent: string,
  fromUrl: string,
  toUrl: string,
): Promise<boolean> {
  try {
    await mgr.link({
      server: { name: serverName, spec: nextSpec },
      // biome-ignore lint/suspicious/noExplicitAny: agent is a manifest AgentId key threaded through opaque catalog types
      agent: agent as any,
      allowOverwrite: true,
    })
    logger.info('mcpUrl migration: relinked', {
      serverName,
      agent,
      from: fromUrl,
      to: toUrl,
    })
    return true
  } catch (err) {
    logger.warn('mcpUrl migration: relink failed', {
      serverName,
      agent,
      error: err instanceof Error ? err.message : String(err),
    })
    return false
  }
}

async function rewriteProfiles(
  servers: Awaited<ReturnType<BoundApi['list']>>,
  migratedServerNames: Set<string>,
  targetMcpUrl: string,
  counters: MigrationCounters,
): Promise<void> {
  const profileNames = await listFiles(AGENTS_SUBDIR)
  for (const name of profileNames) {
    await rewriteOneProfile(
      name,
      servers,
      migratedServerNames,
      targetMcpUrl,
      counters,
    )
  }
}

async function rewriteOneProfile(
  fileName: string,
  servers: Awaited<ReturnType<BoundApi['list']>>,
  migratedServerNames: Set<string>,
  targetMcpUrl: string,
  counters: MigrationCounters,
): Promise<void> {
  const file = `${AGENTS_SUBDIR}/${fileName}`
  try {
    const raw = await readJsonRaw(file)
    const parseResult = storedAgentProfileSchema.safeParse(raw)
    if (parseResult.success) {
      await maybeUpdateProfile(
        file,
        parseResult.data,
        servers,
        migratedServerNames,
        targetMcpUrl,
      )
      return
    }
    // Parse failed. If the harness value is present but no longer in
    // the enum, this is a retired-harness profile from before an
    // upgrade; quarantine it. Anything else (corrupt file, missing
    // harness field, etc.) is a real failure the operator should see.
    const harness = extractHarnessValue(raw)
    if (harness !== null) {
      await quarantineRetiredProfile(file, fileName, harness)
      return
    }
    counters.failed++
    logger.warn('mcpUrl migration: profile parse failed', {
      file,
      error: parseResult.error.message,
    })
  } catch (err) {
    counters.failed++
    logger.warn('mcpUrl migration: profile rewrite failed', {
      file,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}

async function maybeUpdateProfile(
  file: string,
  profile: StoredAgentProfile,
  servers: Awaited<ReturnType<BoundApi['list']>>,
  migratedServerNames: Set<string>,
  targetMcpUrl: string,
): Promise<void> {
  if (profile.mcpUrl === targetMcpUrl) return
  const serverExists = servers.some((s) => s.name === profile.slug)
  if (serverExists && !migratedServerNames.has(profile.slug)) {
    logger.info(
      'mcpUrl migration: skipped profile whose manifest relink failed',
      { slug: profile.slug, file },
    )
    return
  }
  const updated: StoredAgentProfile = { ...profile, mcpUrl: targetMcpUrl }
  await writeJson(file, updated, storedAgentProfileSchema)
  logger.info('mcpUrl migration: updated stored profile', {
    slug: profile.slug,
    from: profile.mcpUrl,
    to: targetMcpUrl,
  })
}

function extractHarnessValue(raw: unknown): string | null {
  if (raw && typeof raw === 'object' && 'harness' in raw) {
    const value = (raw as { harness: unknown }).harness
    if (typeof value === 'string') return value
  }
  return null
}

async function quarantineRetiredProfile(
  relPath: string,
  fileName: string,
  harness: string,
): Promise<void> {
  try {
    await ensureDir(RETIRED_SUBDIR)
    const source = resolveClawServerPath(relPath)
    const dest = resolveClawServerPath(RETIRED_SUBDIR, fileName)
    await rename(source, dest)
    logger.info('mcpUrl migration: quarantined retired-harness profile', {
      from: relPath,
      to: `${RETIRED_SUBDIR}/${fileName}`,
      harness,
    })
  } catch (err) {
    logger.warn('mcpUrl migration: could not quarantine retired profile', {
      file: relPath,
      harness,
      error: err instanceof Error ? err.message : String(err),
    })
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
  // Rewrite ONLY the first HTTP-like arg to match `extractSpecUrl`'s
  // `Array.find` semantics. `rewriteSpecUrl` used to map every arg,
  // which would silently corrupt a stdio spec that carried more than
  // one HTTP arg. Current harness catalog only writes one URL per
  // stdio spec, but the symmetric fix removes future-catalog risk.
  const args = spec.args ?? []
  const firstUrlIdx = args.findIndex((a) => /^https?:\/\//.test(a))
  if (firstUrlIdx === -1) return { ...spec }
  const nextArgs = args.slice()
  nextArgs[firstUrlIdx] = newUrl
  return { ...spec, args: nextArgs }
}
