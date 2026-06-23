/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Database as BunDatabase } from 'bun:sqlite'
import { existsSync, mkdirSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { type BunSQLiteDatabase, drizzle } from 'drizzle-orm/bun-sqlite'
import { migrate } from 'drizzle-orm/bun-sqlite/migrator'
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

/** Opens BrowserOS SQLite and applies checked-in Drizzle migrations before callers use the DB. */
export function openBrowserOsDatabase(options: OpenDbOptions): DbHandle {
  const migrationsDir = resolveMigrationsDir(options)
  mkdirSync(dirname(options.dbPath), { recursive: true })

  const sqlite = new BunDatabase(options.dbPath)
  sqlite.exec('PRAGMA journal_mode = WAL')
  sqlite.exec('PRAGMA foreign_keys = ON')

  const db = drizzle(sqlite, { schema })
  if (options.runMigrations !== false) {
    if (migrationsDir) {
      migrate(db, { migrationsFolder: migrationsDir })
    } else {
      logger.warn('Drizzle migrations unavailable; bootstrapping current schema', {
        dbPath: options.dbPath,
      })
      bootstrapCurrentSchema(sqlite)
    }
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
      'Configured Drizzle migrations directory is missing or incomplete; bootstrapping current schema',
      { migrationsDir: options.migrationsDir },
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
  const journal = readDrizzleJournal(join(migrationsDir, 'meta', '_journal.json'))
  if (!journal) return false

  const journalTags = new Set(journal.entries.map((entry) => entry.tag))
  if (
    !currentMigrationHistory.every((migration) =>
      journalTags.has(migration.tag),
    )
  ) {
    return false
  }

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

/** Creates the current schema when packaged builds lack migration files, and marks those migrations applied. */
function bootstrapCurrentSchema(sqlite: BunDatabase): void {
  sqlite.exec('BEGIN')
  try {
    for (const statement of currentSchemaStatements) {
      sqlite.exec(statement)
    }
    const insertMigration = sqlite.prepare(`
      INSERT INTO __drizzle_migrations ("hash", "created_at")
      SELECT ?, ?
      WHERE NOT EXISTS (
        SELECT 1 FROM __drizzle_migrations
        WHERE created_at = ?
      )
    `)
    for (const migration of currentMigrationHistory) {
      insertMigration.run(
        migration.hash,
        migration.createdAt,
        migration.createdAt,
      )
    }
    sqlite.exec('COMMIT')
  } catch (error) {
    sqlite.exec('ROLLBACK')
    throw error
  }
}

const currentMigrationHistory = [
  {
    tag: '0000_zippy_psylocke',
    hash: 'aadfc2e86410febb11a974d25d99d5f7196aa797d9635ced9a18cd4eeb503b61',
    createdAt: 1777750582590,
  },
  {
    tag: '0001_lazy_orphan',
    hash: '19e693f7b1adcd1d932fa6cf5638b5b158c66ea5de4f154bc59311f4d6f71261',
    createdAt: 1777752799806,
  },
  {
    tag: '0002_chemical_whirlwind',
    hash: '02b11bf1dc34a5a289efd216233a48f0b7b950cfc33eaa7ebe6dcbb15d07f75c',
    createdAt: 1777902205667,
  },
  {
    tag: '0003_scrub_hermes_credentials',
    hash: '34387e59aa1f0d6dc44c95836d2363b72982663c50d05d0c67ee58c211209f52',
    createdAt: 1781916712443,
  },
]

// TODO(nikhil): Remove this fallback once Windows/Linux packaging always includes Drizzle migrations.
const currentSchemaStatements = [
  `
    CREATE TABLE IF NOT EXISTS agent_definitions (
      id text PRIMARY KEY NOT NULL,
      name text NOT NULL,
      adapter text NOT NULL,
      model_id text NOT NULL,
      reasoning_effort text NOT NULL,
      permission_mode text DEFAULT 'approve-all' NOT NULL,
      session_key text NOT NULL,
      pinned integer DEFAULT false NOT NULL,
      adapter_config_json text,
      created_at integer NOT NULL,
      updated_at integer NOT NULL
    )
  `,
  `
    CREATE UNIQUE INDEX IF NOT EXISTS agent_definitions_session_key_unique
    ON agent_definitions (session_key)
  `,
  `
    CREATE INDEX IF NOT EXISTS agent_definitions_updated_at_idx
    ON agent_definitions (updated_at)
  `,
  `
    CREATE INDEX IF NOT EXISTS agent_definitions_adapter_updated_at_idx
    ON agent_definitions (adapter, updated_at)
  `,
  `
    CREATE TABLE IF NOT EXISTS oauth_tokens (
      browseros_id text NOT NULL,
      provider text NOT NULL,
      access_token text NOT NULL,
      refresh_token text NOT NULL,
      expires_at integer NOT NULL,
      email text,
      account_id text,
      updated_at integer NOT NULL,
      PRIMARY KEY (browseros_id, provider)
    )
  `,
  `
    CREATE INDEX IF NOT EXISTS oauth_tokens_browseros_id_idx
    ON oauth_tokens (browseros_id)
  `,
  `
    CREATE TABLE IF NOT EXISTS produced_files (
      id text PRIMARY KEY NOT NULL,
      agent_definition_id text NOT NULL,
      session_key text NOT NULL,
      turn_id text NOT NULL,
      turn_prompt text NOT NULL,
      path text NOT NULL,
      size integer NOT NULL,
      mtime_ms integer NOT NULL,
      created_at integer NOT NULL,
      detected_by text DEFAULT 'diff' NOT NULL,
      FOREIGN KEY (agent_definition_id)
        REFERENCES agent_definitions(id)
        ON UPDATE no action
        ON DELETE cascade
    )
  `,
  `
    CREATE UNIQUE INDEX IF NOT EXISTS produced_files_agent_path_unique
    ON produced_files (agent_definition_id, path)
  `,
  `
    CREATE INDEX IF NOT EXISTS produced_files_agent_created_idx
    ON produced_files (agent_definition_id, created_at)
  `,
  `
    CREATE INDEX IF NOT EXISTS produced_files_turn_idx
    ON produced_files (turn_id)
  `,
  `
    CREATE INDEX IF NOT EXISTS produced_files_session_idx
    ON produced_files (session_key)
  `,
  `
    UPDATE agent_definitions
    SET adapter_config_json = NULL
    WHERE adapter = 'hermes' AND adapter_config_json IS NOT NULL
  `,
  `
    CREATE TABLE IF NOT EXISTS __drizzle_migrations (
      id SERIAL PRIMARY KEY,
      hash text NOT NULL,
      created_at numeric
    )
  `,
]
