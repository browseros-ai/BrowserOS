/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { Database as BunDatabase } from 'bun:sqlite'
import { afterEach, describe, expect, it } from 'bun:test'
import { createHash } from 'node:crypto'
import { existsSync, mkdirSync, mkdtempSync, readFileSync } from 'node:fs'
import { rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { closeDb, initializeDb } from '../../../src/lib/db'
import { providers } from '../../../src/lib/db/schema'

describe('database initialization', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    closeDb()
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('creates the parent directory, opens sqlite, and runs migrations', () => {
    const dir = mkTempDir()
    const dbPath = join(dir, 'nested', 'browseros.sqlite')

    const handle = initializeDb({ dbPath })
    const rows = handle.db.select().from(providers).all()

    expect(existsSync(dbPath)).toBe(true)
    expect(rows).toEqual([])
  })

  it('is idempotent when initialized twice for the same path', () => {
    const dir = mkTempDir()
    const dbPath = join(dir, 'browseros.sqlite')

    const first = initializeDb({ dbPath })
    const second = initializeDb({ dbPath })

    expect(second).toBe(first)
  })

  it.each([false, true])(
    'upgrades an existing provider without changing credentials or selection (missing migrations: %s)',
    (missingMigrations) => {
      const dbPath = join(mkTempDir(), 'browseros.sqlite')
      const old = initializeDb({ dbPath })
      old.sqlite.exec('ALTER TABLE providers DROP COLUMN headers')
      old.sqlite
        .query('DELETE FROM __drizzle_migrations WHERE created_at = ?')
        .run(expectedMigrationHistory.at(-1).createdAt)
      old.sqlite.exec(
        "INSERT INTO providers (id, kind, type, name, model_id, context_window, api_key, is_default, created_at, updated_at) VALUES ('existing', 'llm', 'openai', 'Existing', 'model', 128000, 'local-key', 1, 1, 1)",
      )
      closeDb()

      const upgraded = initializeDb({
        dbPath,
        ...(missingMigrations && {
          migrationsDir: join(mkTempDir(), 'missing'),
        }),
      })
      expect(upgraded.db.select().from(providers).get()).toMatchObject({
        id: 'existing',
        headers: null,
        apiKey: 'local-key',
        isDefault: true,
      })
    },
  )

  it('bootstraps the current schema when migration files are unavailable', () => {
    const dir = mkTempDir()
    const handle = initializeDb({
      dbPath: join(dir, 'browseros.sqlite'),
      migrationsDir: join(dir, 'missing-migrations'),
    })

    expectCurrentSchema(handle)
    expect(handle.db.select().from(providers).all()).toEqual([])
  })

  it('bootstraps the current schema when a migration directory is empty', () => {
    const dir = mkTempDir()
    const migrationsDir = join(dir, 'empty-migrations')
    mkdirSync(migrationsDir)

    const handle = initializeDb({
      dbPath: join(dir, 'browseros.sqlite'),
      migrationsDir,
    })

    expect(handle.migrationsDir).toBe(null)
    expectCurrentSchema(handle)
    expect(handle.db.select().from(providers).all()).toEqual([])
  })

  it('skips empty packaged migration resources', () => {
    const dir = mkTempDir()
    const resourcesDir = join(dir, 'resources')
    const packagedMigrationsDir = join(resourcesDir, 'db', 'migrations')
    mkdirSync(packagedMigrationsDir, { recursive: true })

    const handle = initializeDb({
      dbPath: join(dir, 'browseros.sqlite'),
      resourcesDir,
    })

    expect(handle.migrationsDir).not.toBe(packagedMigrationsDir)
    expect(handle.db.select().from(providers).all()).toEqual([])
  })

  it('does not rerun old migrations after fallback schema bootstrap', () => {
    const dir = mkTempDir()
    const dbPath = join(dir, 'browseros.sqlite')

    initializeDb({
      dbPath,
      migrationsDir: join(dir, 'missing-migrations'),
    })
    closeDb()

    expect(() => initializeDb({ dbPath })).not.toThrow()
  })

  it('deletes legacy agent records instead of migrating them', () => {
    const dir = mkTempDir()
    const dbPath = join(dir, 'browseros.sqlite')
    const sqlite = new BunDatabase(dbPath)
    sqlite.exec(`
      CREATE TABLE agent_definitions (
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
      );
      CREATE TABLE __drizzle_migrations (
        id SERIAL PRIMARY KEY,
        hash text NOT NULL,
        created_at numeric
      );
      CREATE TABLE produced_files (id text PRIMARY KEY NOT NULL);
    `)
    for (const migration of expectedMigrationHistory.slice(0, 4)) {
      sqlite
        .prepare(
          'INSERT INTO __drizzle_migrations ("hash", "created_at") VALUES (?, ?)',
        )
        .run(migration.hash, migration.createdAt)
    }
    sqlite
      .prepare(
        `
          INSERT INTO agent_definitions (
            id,
            name,
            adapter,
            model_id,
            reasoning_effort,
            permission_mode,
            session_key,
            pinned,
            adapter_config_json,
            created_at,
            updated_at
          ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        `,
      )
      .run(
        'legacy-claude',
        'Legacy Claude',
        'claude',
        'default',
        'medium',
        'approve-all',
        'agent:legacy-claude:main',
        false,
        '{"apiKey":"secret"}',
        1000,
        1000,
      )
    sqlite.close()

    const handle = initializeDb({ dbPath })
    const legacyTable = handle.sqlite
      .query<{ name: string }, []>(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'agent_definitions'",
      )
      .get()

    expect(legacyTable).toBeNull()
    expect(handle.db.select().from(providers).all()).toEqual([])
  })

  function expectCurrentSchema(handle: ReturnType<typeof initializeDb>): void {
    const tables = handle.sqlite
      .query<{ name: string }, []>(
        `
          SELECT name FROM sqlite_master
          WHERE type = 'table'
            AND name IN (
              'providers',
              'scheduled_jobs',
              'scheduled_job_runs',
              'oauth_tokens',
              '__drizzle_migrations'
            )
          ORDER BY name
        `,
      )
      .all()
      .map((row) => row.name)

    // The fallback has to produce the schema as it stands after every
    // migration, so the two split provider tables are absent and the unified
    // one is present. It drifted behind once already, which is what this list
    // is here to catch.
    expect(tables).toEqual([
      '__drizzle_migrations',
      'oauth_tokens',
      'providers',
      'scheduled_job_runs',
      'scheduled_jobs',
    ])
    const migrations = handle.sqlite
      .query<{ hash: string; createdAt: number }, []>(
        `
          SELECT hash, created_at AS createdAt
          FROM __drizzle_migrations
          ORDER BY created_at
        `,
      )
      .all()

    expect(migrations).toEqual(expectedMigrationHistory)
  }

  function mkTempDir(): string {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-db-test-'))
    tempDirs.push(dir)
    return dir
  }
})

/**
 * Derived from the journal rather than transcribed.
 *
 * The bootstrap fallback carries its own copy of this history, and a hand
 * written duplicate here is what let that copy fall four migrations behind
 * without any test noticing. Reading the journal and hashing the files means
 * adding a migration and forgetting the fallback now fails.
 */
const expectedMigrationHistory = JSON.parse(
  readFileSync(
    join(import.meta.dir, '../../../src/lib/db/migrations/meta/_journal.json'),
    'utf8',
  ),
).entries.map((entry: { tag: string; when: number }) => ({
  hash: createHash('sha256')
    .update(
      readFileSync(
        join(
          import.meta.dir,
          `../../../src/lib/db/migrations/${entry.tag}.sql`,
        ),
      ),
    )
    .digest('hex'),
  createdAt: entry.when,
}))
