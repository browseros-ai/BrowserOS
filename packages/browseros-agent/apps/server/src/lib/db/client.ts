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
      logger.warn(
        'Drizzle migrations unavailable; bootstrapping current schema',
        {
          dbPath: options.dbPath,
        },
      )
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
  const journal = readDrizzleJournal(
    join(migrationsDir, 'meta', '_journal.json'),
  )
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
    const providerColumns = sqlite
      .query<{ name: string }, []>('PRAGMA table_info(providers)')
      .all()
    if (!providerColumns.some((column) => column.name === 'headers')) {
      sqlite.exec('ALTER TABLE providers ADD COLUMN headers text')
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
  {
    tag: '0004_sparkling_carnage',
    hash: '76d3a9d6c383995df79b6d8f66ae1bedd0b97b1f44e90c047d8853666bbcc9fd',
    createdAt: 1785893663690,
  },
  {
    tag: '0005_yellow_riptide',
    hash: '44a8d4afc62cc58f0f958f633e5262331370d1e1538981b69c1ec2cb807a3154',
    createdAt: 1785900211901,
  },
  {
    tag: '0006_add_conversations',
    hash: 'e9a01f94d41f7718c66039a8483302f6db7c7de946f99987a6dd2e78613bce90',
    createdAt: 1786538823114,
  },
  {
    tag: '0007_add_custom_acp_agents',
    hash: '561eb1075d7487ffe0394e587eef7ba35ccd892e3e3b53acace579cb0477576b',
    createdAt: 1787580067090,
  },
  {
    tag: '0008_add_llm_providers_and_scheduled_jobs',
    hash: '1e36c60be880a222ae150858c5248a433556bd974c52164c42d1955e84ba6606',
    createdAt: 1788319873053,
  },
  {
    tag: '0009_add_scheduled_job_runs',
    hash: '188a9503d889be46926bd6d4d660a1c016c90fac71447c25eec73e421b90fc96',
    createdAt: 1788413695569,
  },
  {
    tag: '0010_add_unified_providers_table',
    hash: '9e5731582228e0de16bb28f5465cfd62ec2662822f43122f84b8039dd2c0cf0b',
    createdAt: 1788426799725,
  },
  {
    tag: '0011_drop_split_provider_tables',
    hash: 'eb0fa2687c80caf919248f28cda5cd955e01a671b2104308b4d04ec55d450611',
    createdAt: 1788426855683,
  },
  {
    tag: '0012_add_provider_headers',
    hash: '5e1894d0aebf4a5b708425f565795b01e4efdb997a1e1e6fc6479f229bd022da',
    createdAt: 1788724664440,
  },
]

// TODO(nikhil): Remove this fallback once Windows/Linux packaging always includes Drizzle migrations.
const currentSchemaStatements = [
  `
    CREATE TABLE IF NOT EXISTS providers (
      id text PRIMARY KEY NOT NULL,
      profile_id text,
      kind text NOT NULL,
      type text NOT NULL,
      name text NOT NULL,
      model_id text,
      reasoning_effort text,
      is_default integer DEFAULT false NOT NULL,
      created_at integer NOT NULL,
      updated_at integer NOT NULL,
      base_url text,
      headers text,
      supports_images integer DEFAULT true NOT NULL,
      context_window integer,
      temperature real DEFAULT 0.2 NOT NULL,
      api_key text,
      access_key_id text,
      secret_access_key text,
      session_token text,
      resource_name text,
      region text,
      reasoning_summary text,
      working_directory text,
      custom_config text,
      CONSTRAINT "providers_llm_requires_model_and_context" CHECK("providers"."kind" <> 'llm' OR ("providers"."model_id" IS NOT NULL AND "providers"."context_window" IS NOT NULL))
    )
  `,
  `
    CREATE INDEX IF NOT EXISTS providers_profile_id_idx
    ON providers (profile_id)
  `,
  `
    CREATE INDEX IF NOT EXISTS providers_kind_updated_at_idx
    ON providers (kind, updated_at)
  `,
  `
    CREATE UNIQUE INDEX IF NOT EXISTS providers_one_default
    ON providers (is_default) WHERE "providers"."is_default" = 1
  `,
  `
    CREATE TABLE IF NOT EXISTS scheduled_jobs (
      id text PRIMARY KEY NOT NULL,
      profile_id text,
      name text NOT NULL,
      query text NOT NULL,
      schedule_type text NOT NULL,
      schedule_time text,
      schedule_interval integer,
      enabled integer DEFAULT true NOT NULL,
      provider_id text,
      last_run_at integer,
      created_at integer NOT NULL,
      updated_at integer NOT NULL,
      FOREIGN KEY (provider_id) REFERENCES providers(id) ON UPDATE no action ON DELETE set null
    )
  `,
  `
    CREATE INDEX IF NOT EXISTS scheduled_jobs_profile_id_idx
    ON scheduled_jobs (profile_id)
  `,
  `
    CREATE INDEX IF NOT EXISTS scheduled_jobs_enabled_idx
    ON scheduled_jobs (enabled)
  `,
  `
    CREATE TABLE IF NOT EXISTS scheduled_job_runs (
      id text PRIMARY KEY NOT NULL,
      profile_id text,
      job_id text NOT NULL,
      status text NOT NULL,
      started_at integer NOT NULL,
      completed_at integer,
      result text,
      final_result text,
      execution_log text,
      tool_calls text,
      error text,
      created_at integer NOT NULL,
      updated_at integer NOT NULL,
      FOREIGN KEY (job_id) REFERENCES scheduled_jobs(id) ON UPDATE no action ON DELETE cascade
    )
  `,
  `
    CREATE INDEX IF NOT EXISTS scheduled_job_runs_job_id_idx
    ON scheduled_job_runs (job_id)
  `,
  `
    CREATE INDEX IF NOT EXISTS scheduled_job_runs_started_at_idx
    ON scheduled_job_runs (started_at)
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
    CREATE TABLE IF NOT EXISTS conversations (
      id text PRIMARY KEY NOT NULL,
      messages text NOT NULL,
      last_user_message text,
      origin text,
      target_type text NOT NULL,
      agent_id text,
      last_messaged_at integer NOT NULL,
      created_at integer NOT NULL,
      updated_at integer NOT NULL
    )
  `,
  `
    CREATE INDEX IF NOT EXISTS conversations_last_messaged_at_idx
    ON conversations (last_messaged_at)
  `,
  `
    CREATE TABLE IF NOT EXISTS __drizzle_migrations (
      id SERIAL PRIMARY KEY,
      hash text NOT NULL,
      created_at numeric
    )
  `,
]
