/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * One-shot migration that runs on startup of the merged runtime.
 *
 * The cockpit moved from binding its own port (9200) to mounting
 * inside `@browseros/server`'s HTTP runtime under a `/cockpit`
 * prefix. Every profile saved before this change carries an
 * `mcpUrl` like `http://127.0.0.1:9200/mcp/<slug>` which now 404s,
 * and every harness config row written by `agent-mcp-manager`
 * points at the same dead URL. The migration walks the profile
 * directory, rewrites `mcpUrl` to the new `buildMcpUrl(slug)`
 * shape, and re-installs the harness entry so it picks up the new
 * value.
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
      // URL. Both are best-effort; a failed uninstall (e.g. the
      // user removed the entry by hand) does not abort the install.
      await uninstallForAgent({ slug: profile.slug, harness: profile.harness })
      await installForAgent({
        slug: updated.slug,
        mcpUrl: updated.mcpUrl,
        harness: updated.harness,
      })
      migrated++
      logger.info('migrated cockpit mcpUrl after runtime merge', {
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
