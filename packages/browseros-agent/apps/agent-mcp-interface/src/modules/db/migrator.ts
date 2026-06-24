/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Programmatic migration runner for the cockpit audit DB. Called from
 * `db.ts` on first construction so a fresh BrowserOS install picks up
 * every migration in `drizzle/` before the first write.
 *
 * Resolves the `drizzle/` folder relative to this source file via
 * `import.meta.dir`, so the migration set ships with the package and
 * runs the same in production and tests.
 */

import { resolve } from 'node:path'
import { migrate } from 'drizzle-orm/bun-sqlite/migrator'
import type { AuditDb } from './db'

export function runMigrations(db: AuditDb): void {
  // From `src/modules/db/migrator.ts` to `<pkg>/drizzle` is three
  // levels up (modules -> src -> package root) plus the `drizzle`
  // folder.
  const migrationsFolder = resolve(import.meta.dir, '../../../drizzle')
  migrate(db, { migrationsFolder })
}
