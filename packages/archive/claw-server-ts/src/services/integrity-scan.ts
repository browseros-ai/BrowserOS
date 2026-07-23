/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Boot-time self-heal for MCP entries. Diffs the workspace manifest
 * against on-disk agent config files via `rescan`, and for every
 * drifted or missing entry re-links the manifest-stored spec so the
 * config file catches up.
 *
 * "Drifted" = manifest says agent X has server Y linked to configPath
 * Z, the file exists, but the emitter can't find an entry under that
 * name (someone edited it out; another tool trimmed it; a partial
 * write left it half-there). "Missing" = the config file itself is
 * gone. Both are recoverable by re-`link`ing from the spec the
 * manifest already remembers.
 *
 * Per-entry failures log a warn and continue; a single bad path does
 * not block the rest of the sweep.
 */

import { logger } from '../lib/logger'
import { getMcpManager } from '../lib/mcp-manager'

export interface IntegrityScanOutcome {
  verified: number
  drifted: number
  missing: number
  healed: number
  failed: number
}

export async function runIntegrityScan(): Promise<IntegrityScanOutcome> {
  const mgr = getMcpManager()
  const report = await mgr.rescan()
  const servers = await mgr.list()
  const specByName = new Map(servers.map((s) => [s.name, s.spec]))

  const toHeal = [...report.drifted, ...report.missing]
  let healed = 0
  let failed = 0

  for (const entry of toHeal) {
    const spec = specByName.get(entry.serverName)
    if (!spec) {
      failed++
      logger.warn('integrity scan: no manifest spec for drifted entry', {
        serverName: entry.serverName,
        agent: entry.agent,
        reason: entry.reason,
      })
      continue
    }
    try {
      await mgr.link({
        server: { name: entry.serverName, spec },
        agent: entry.agent,
        scope: entry.scope,
        allowOverwrite: true,
      })
      healed++
      logger.info('integrity scan: healed drifted entry', {
        serverName: entry.serverName,
        agent: entry.agent,
        reason: entry.reason,
      })
    } catch (err) {
      failed++
      logger.warn('integrity scan: heal failed', {
        serverName: entry.serverName,
        agent: entry.agent,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  return {
    verified: report.verified.length,
    drifted: report.drifted.length,
    missing: report.missing.length,
    healed,
    failed,
  }
}
