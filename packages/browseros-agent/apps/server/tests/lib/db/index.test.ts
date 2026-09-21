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

  it('refuses to start when migrations are unavailable', () => {
    const dir = mkTempDir()

    expect(() =>
      initializeDb({
        dbPath: join(dir, 'browseros.sqlite'),
        migrationsDir: join(dir, 'missing-migrations'),
      }),
    ).toThrow(/migrations are unavailable/)
  })

  it('falls back to the source migrations when packaged resources are empty', () => {
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

  it('upgrades an existing provider without changing credentials or selection', () => {
    const dbPath = join(mkTempDir(), 'browseros.sqlite')
    const old = initializeDb({ dbPath })
    // Simulate a database one migration behind: drop the column 0012 adds and
    // un-record 0012 so the next launch re-applies it.
    old.sqlite.exec('ALTER TABLE providers DROP COLUMN headers')
    old.sqlite
      .query('DELETE FROM __drizzle_migrations WHERE created_at = ?')
      .run(expectedMigrationHistory.at(-1).createdAt)
    old.sqlite.exec(
      "INSERT INTO providers (id, kind, type, name, model_id, context_window, api_key, is_default, created_at, updated_at) VALUES ('existing', 'llm', 'openai', 'Existing', 'model', 128000, 'local-key', 1, 1, 1)",
    )
    closeDb()

    const upgraded = initializeDb({ dbPath })
    expect(upgraded.db.select().from(providers).get()).toMatchObject({
      id: 'existing',
      headers: null,
      apiKey: 'local-key',
      isDefault: true,
    })
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

  it('recreates a table dropped after its migrations were already stamped', () => {
    const dbPath = join(mkTempDir(), 'browseros.sqlite')
    const first = initializeDb({ dbPath })
    first.sqlite.exec(
      "INSERT INTO conversations (id, messages, target_type, last_messaged_at, created_at, updated_at) VALUES ('c1', '[]', 'agent', 1, 1, 1)",
    )
    // The ledger stays fully stamped but a table is gone. Drizzle's
    // timestamp-gated migrator will not recreate it on the next launch, so only
    // the recovery can.
    first.sqlite.exec('DROP TABLE providers')
    closeDb()

    const repaired = initializeDb({ dbPath })

    expect(repaired.db.select().from(providers).all()).toEqual([])
    expect(
      repaired.sqlite
        .query<{ id: string }, []>('SELECT id FROM conversations')
        .all(),
    ).toEqual([{ id: 'c1' }])
  })

  it('recovers a drifted database before a migration references the missing table', () => {
    const dbPath = join(mkTempDir(), 'browseros.sqlite')
    const old = initializeDb({ dbPath })
    // A drifted database that lost `providers` yet recorded migrations only
    // through the one before 0012's `ALTER TABLE providers ADD headers`. On the
    // next launch migrate() runs that ALTER and throws on the missing table; the
    // recovery kicks in from the same migrations.
    old.sqlite.exec('DROP TABLE providers')
    old.sqlite
      .query('DELETE FROM __drizzle_migrations WHERE created_at = ?')
      .run(expectedMigrationHistory.at(-1).createdAt)
    closeDb()

    const repaired = initializeDb({ dbPath })

    expect(repaired.db.select().from(providers).all()).toEqual([])
  })

  function mkTempDir(): string {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-db-test-'))
    tempDirs.push(dir)
    return dir
  }
})

/**
 * Derived from the journal rather than transcribed, so a new migration cannot
 * silently fall out of sync with what the tests exercise.
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
