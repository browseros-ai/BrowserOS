/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Database as BunDatabase } from 'bun:sqlite'
import { existsSync, mkdirSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { getTableName, is } from 'drizzle-orm'
import { type BunSQLiteDatabase, drizzle } from 'drizzle-orm/bun-sqlite'
import { migrate } from 'drizzle-orm/bun-sqlite/migrator'
import { SQLiteTable } from 'drizzle-orm/sqlite-core'
import { logger } from '../logger'
import * as schema from './schema'

export type BrowserOsDatabase = BunSQLiteDatabase<typeof schema>

interface DrizzleJournalEntry {
  tag: string
}

export interface DbHandle {
  path: string
  migrationsDir: string | null
  sqlite: BunDatabase
  db: BrowserOsDatabase
}

export interface OpenDbOptions {
  dbPath: string
  resourcesDir?: string
  migrationsDir?: string
  runMigrations?: boolean
}

const sourceMigrationsDir = fileURLToPath(
  new URL('./migrations', import.meta.url),
)

/** Opens BrowserOS SQLite and applies the checked-in Drizzle migrations before callers use the DB. */
export function openBrowserOsDatabase(options: OpenDbOptions): DbHandle {
  const migrationsDir = resolveMigrationsDir(options)
  mkdirSync(dirname(options.dbPath), { recursive: true })

  const sqlite = new BunDatabase(options.dbPath)
  sqlite.exec('PRAGMA journal_mode = WAL')
  sqlite.exec('PRAGMA foreign_keys = ON')

  const db = drizzle(sqlite, { schema })
  if (options.runMigrations !== false) {
    if (!migrationsDir) {
      // Migrations ship with every build and the server cannot run without
      // them, so a missing set is a fatal packaging error, not something to
      // paper over with a parallel schema that can silently drift.
      throw new Error(
        'BrowserOS database migrations are unavailable; refusing to start with an unmigrated database',
      )
    }
    try {
      migrate(db, { migrationsFolder: migrationsDir })
    } catch (error) {
      // A drifted database (its ledger says migrations are applied while tables
      // are missing) makes a migration reference an absent table, e.g. 0012's
      // ALTER TABLE providers, which aborts migrate(). Recover from the same
      // migrations and continue; re-throw anything else so a real migration bug
      // is not masked.
      if (missingRequiredTables(sqlite).length === 0) throw error
      recoverDriftedSchema(sqlite, migrationsDir)
    }
    // Safeguard for a ledger that is fully stamped yet missing tables, where
    // migrate() succeeds by doing nothing.
    recoverDriftedSchema(sqlite, migrationsDir)
  }

  return {
    path: options.dbPath,
    migrationsDir,
    sqlite,
    db,
  }
}

/** Resolves migrations from explicit test paths, packaged resources, or the source tree. */
export function resolveMigrationsDir(
  options: Pick<OpenDbOptions, 'migrationsDir' | 'resourcesDir'> = {},
): string | null {
  if (options.migrationsDir) {
    if (hasCompleteMigrationSet(options.migrationsDir)) {
      return options.migrationsDir
    }
    logger.warn(
      'Configured Drizzle migrations directory is missing or incomplete',
      {
        migrationsDir: options.migrationsDir,
      },
    )
    return null
  }

  const candidates = [
    options.resourcesDir
      ? join(options.resourcesDir, 'db', 'migrations')
      : null,
    sourceMigrationsDir,
  ].filter((candidate): candidate is string => Boolean(candidate))

  for (const candidate of candidates) {
    if (hasCompleteMigrationSet(candidate)) return candidate
  }

  return null
}

/** Accepts only migration folders Drizzle can read without filesystem errors. */
function hasCompleteMigrationSet(migrationsDir: string): boolean {
  const journal = readDrizzleJournal(
    join(migrationsDir, 'meta', '_journal.json'),
  )
  if (!journal) return false
  if (journal.entries.length === 0) return false

  return journal.entries.every((entry) =>
    existsSync(join(migrationsDir, `${entry.tag}.sql`)),
  )
}

function readDrizzleJournal(
  journalPath: string,
): { entries: DrizzleJournalEntry[] } | null {
  if (!existsSync(journalPath)) return null

  try {
    const journal = JSON.parse(readFileSync(journalPath, 'utf8')) as unknown
    if (!isDrizzleJournal(journal)) return null
    return journal
  } catch {
    return null
  }
}

function isDrizzleJournal(
  value: unknown,
): value is { entries: DrizzleJournalEntry[] } {
  return (
    typeof value === 'object' &&
    value !== null &&
    'entries' in value &&
    Array.isArray(value.entries) &&
    value.entries.every(
      (entry) =>
        typeof entry === 'object' &&
        entry !== null &&
        'tag' in entry &&
        typeof entry.tag === 'string',
    )
  )
}

/** Table names the ORM schema declares, so drift checks never lag a hand-list. */
function requiredTableNames(): string[] {
  const names: string[] = []
  for (const value of Object.values(schema)) {
    if (is(value, SQLiteTable)) names.push(getTableName(value))
  }
  return names
}

/** Schema tables absent from the database, the signal that its schema drifted. */
function missingRequiredTables(sqlite: BunDatabase): string[] {
  const present = new Set(
    sqlite
      .query<{ name: string }, []>(
        "SELECT name FROM sqlite_master WHERE type = 'table'",
      )
      .all()
      .map((row) => row.name),
  )
  return requiredTableNames().filter((name) => !present.has(name))
}

interface SqliteObject {
  type: string
  name: string
  tbl_name: string
  sql: string
}

/**
 * Recovers a schema that drifted from its migration ledger. Older builds stamped
 * every migration as applied while creating only part of the schema, and
 * Drizzle's timestamp-gated migrator never recreates the tables that were
 * missed. Rebuild the canonical schema from the same migration files in a
 * throwaway in-memory database, then create any table (and its indexes) the real
 * database lacks. The DDL comes from the migrations themselves, never a
 * hand-maintained copy, and existing tables and their rows are untouched.
 */
function recoverDriftedSchema(
  sqlite: BunDatabase,
  migrationsDir: string,
): void {
  const missing = new Set(missingRequiredTables(sqlite))
  if (missing.size === 0) return
  logger.warn(
    'Schema drift detected; recreating missing tables from migrations',
    {
      missing: [...missing],
    },
  )

  const canonical = new BunDatabase(':memory:')
  try {
    migrate(drizzle(canonical), { migrationsFolder: migrationsDir })
    const objects = canonical
      .query<SqliteObject, []>(
        'SELECT type, name, tbl_name, sql FROM sqlite_master WHERE sql IS NOT NULL',
      )
      .all()

    sqlite.exec('BEGIN')
    try {
      for (const object of objects) {
        if (object.type === 'table' && missing.has(object.name)) {
          sqlite.exec(object.sql)
        }
      }
      for (const object of objects) {
        if (object.type === 'index' && missing.has(object.tbl_name)) {
          sqlite.exec(object.sql)
        }
      }
      sqlite.exec('COMMIT')
    } catch (error) {
      sqlite.exec('ROLLBACK')
      throw error
    }
  } finally {
    canonical.close()
  }
}
