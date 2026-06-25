/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * SQLite audit DB. One process-wide singleton, lazily constructed on
 * first use. File lives at `<browserosDir>/mcp-interface/audit.sqlite`
 * so multiple BrowserOS installs (per-user) stay isolated.
 *
 * WAL + NORMAL synchronous is the recommended bun:sqlite profile for
 * write-heavy workloads: writers serialise through one connection,
 * readers run in parallel without blocking.
 */

import { Database } from 'bun:sqlite'
import { mkdirSync } from 'node:fs'
import { dirname } from 'node:path'
import { type BunSQLiteDatabase, drizzle } from 'drizzle-orm/bun-sqlite'
import { resolveClawServerPath } from '../../lib/browseros-dir'
import { runMigrations } from './migrator'
import * as schema from './schema/schema'

export type AuditDb = BunSQLiteDatabase<typeof schema>

let cached: { db: AuditDb; raw: Database } | null = null

function resolveAuditDbPath(): string {
  return resolveClawServerPath('audit.sqlite')
}

export function getAuditDb(): AuditDb {
  if (cached) return cached.db
  const path = resolveAuditDbPath()
  mkdirSync(dirname(path), { recursive: true })
  const raw = new Database(path, { create: true, strict: true })
  raw.run('PRAGMA journal_mode = WAL;')
  raw.run('PRAGMA synchronous = NORMAL;')
  raw.run('PRAGMA foreign_keys = ON;')
  const db = drizzle({ client: raw, schema })
  runMigrations(db)
  cached = { db, raw }
  return db
}

/** Test seam: open a fresh in-memory DB, apply migrations, install as singleton. */
export function setAuditDbForTesting(): AuditDb {
  if (cached) {
    try {
      cached.raw.close()
    } catch {
      // already closed
    }
  }
  const raw = new Database(':memory:', { strict: true })
  raw.run('PRAGMA foreign_keys = ON;')
  const db = drizzle({ client: raw, schema })
  runMigrations(db)
  cached = { db, raw }
  return db
}

/** Test seam: drop the cached singleton so the next caller rebuilds. */
export function resetAuditDbForTesting(): void {
  if (cached) {
    try {
      cached.raw.close()
    } catch {
      // already closed
    }
  }
  cached = null
}
