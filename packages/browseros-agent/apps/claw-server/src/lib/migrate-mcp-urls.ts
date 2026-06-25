/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * One-shot startup migration for stored cockpit MCP URLs.
 *
 * The migration walks the profile directory, rewrites `mcpUrl` to
 * the runtime's current `buildMcpUrl(slug)` shape, and re-installs
 * the harness entry so it picks up the new value.
 *
 * Failures are logged per-profile; one bad file does not abort the
 * sweep. The migration is idempotent: a second run is a no-op once
 * every URL has been refreshed.
 */

import {
  type StoredAgentProfile,
  storedAgentProfileSchema,
} from '../routes/agents/schemas'
import { installForAgent, uninstallForAgent } from '../services/harness-install'
import { logger } from './logger'
import { listFiles, readJson, writeJson } from './storage'

const AGENTS_SUBDIR = 'agents'

export async function migrateMcpUrls(
  buildMcpUrl: (slug: string) => string,
): Promise<{ migrated: number; skipped: number; failed: number }> {
  let migrated = 0
  let skipped = 0
  let failed = 0
  const names = await listFiles(AGENTS_SUBDIR)
  for (const name of names) {
    const file = `${AGENTS_SUBDIR}/${name}`
    try {
      const profile = await readJson(file, storedAgentProfileSchema)
      const next = buildMcpUrl(profile.slug)
      if (profile.mcpUrl === next) {
        skipped++
        continue
      }
      const updated: StoredAgentProfile = { ...profile, mcpUrl: next }
      await writeJson(file, updated, storedAgentProfileSchema)
      // Drop the stale harness entry first, then install the new
      // URL. The uninstall is wrapped in its own try/catch so a
      // throw here (e.g. the user removed the entry by hand and a
      // future agent-mcp-manager build escalates that to an
      // exception) does NOT abort the install. Without this
      // isolation, the profile JSON would carry the new URL while
      // the harness config still points at the dead old one, and
      // the next migration pass would skip the row as "already
      // migrated".
      try {
        await uninstallForAgent({
          slug: profile.slug,
          harness: profile.harness,
        })
      } catch (uninstallErr) {
        logger.warn('migration uninstall step threw; continuing install', {
          file,
          slug: profile.slug,
          error:
            uninstallErr instanceof Error
              ? uninstallErr.message
              : String(uninstallErr),
        })
      }
      await installForAgent({
        slug: updated.slug,
        mcpUrl: updated.mcpUrl,
        harness: updated.harness,
      })
      migrated++
      logger.info('migrated cockpit mcpUrl', {
        slug: profile.slug,
        from: profile.mcpUrl,
        to: next,
      })
    } catch (err) {
      failed++
      logger.warn('failed to migrate cockpit profile mcpUrl', {
        file,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }
  return { migrated, skipped, failed }
}
