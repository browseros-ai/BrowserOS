import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { sql } from 'drizzle-orm'
import { getTableConfig } from 'drizzle-orm/sqlite-core'
import {
  getAuditDb,
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../../src/modules/db/db'
import { resolveMigrationsFolder } from '../../../src/modules/db/migrator'
import { agentSessionEnds } from '../../../src/modules/db/schema/agent-session-ends.sql'
import { agentSessionStarts } from '../../../src/modules/db/schema/agent-session-starts.sql'
import { toolDispatches } from '../../../src/modules/db/schema/tool-dispatches.sql'

describe('audit DB (in-memory test seam)', () => {
  const tempDirs: string[] = []

  beforeEach(() => setAuditDbForTesting())
  afterEach(async () => {
    resetAuditDbForTesting()
    for (const dir of tempDirs.splice(0)) {
      await rm(dir, { recursive: true, force: true })
    }
  })

  it('runs migrations on construction so the tool_dispatches table is queryable', () => {
    const db = getAuditDb()
    const rows = db.select().from(toolDispatches).all()
    expect(rows).toEqual([])
  })

  it('populates createdAt with an epoch-ms default within a few seconds of now', () => {
    const db = getAuditDb()
    db.insert(toolDispatches)
      .values({
        agentId: 'a',
        slug: 'a',
        agentLabel: 'a',
        sessionId: 's',
        toolName: 'tabs',
      })
      .run()
    const row = db.select().from(toolDispatches).get()
    expect(row?.createdAt).toBeGreaterThan(Date.now() - 5_000)
    expect(row?.createdAt).toBeLessThanOrEqual(Date.now() + 1_000)
  })

  // Regression guard for the audit log staying empty on SQLite < 3.42:
  // `unixepoch('subsec')` yields NULL there, so a SQL-only default made
  // every NOT NULL insert fail. createdAt must be defaulted in JS
  // ($defaultFn) so it never depends on the runtime SQLite version. This
  // assertion is version-independent: it fails even on a newer SQLite
  // (e.g. CI) if the SQL-only default is ever reintroduced.
  it('defaults createdAt in JS ($defaultFn), not via a SQLite expression', () => {
    for (const table of [
      toolDispatches,
      agentSessionStarts,
      agentSessionEnds,
    ]) {
      const createdAt = getTableConfig(table).columns.find(
        (column) => column.name === 'created_at',
      )
      expect(createdAt).toBeDefined()
      expect(createdAt?.hasDefault).toBe(true)
      expect(typeof createdAt?.defaultFn).toBe('function')
    }
  })

  it('honours the three indexes defined in the schema', () => {
    const db = getAuditDb()
    const indexes = db
      .all<{ name: string }>(
        sql`SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='tool_dispatches'`,
      )
      .map((r) => r.name)
    expect(indexes).toContain('tool_dispatches_created_at_idx')
    expect(indexes).toContain('tool_dispatches_agent_created_idx')
    expect(indexes).toContain('tool_dispatches_session_idx')
  })

  it('prefers packaged migrations from resources when the set is complete', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'claw-resources-'))
    tempDirs.push(resourcesDir)
    const packagedMigrations = join(resourcesDir, 'db/migrations')
    await writeMigrationSet(packagedMigrations, '0000_packaged')

    expect(resolveMigrationsFolder(resourcesDir)).toBe(packagedMigrations)
  })

  it('falls back to source migrations when packaged resources are incomplete', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'claw-resources-'))
    tempDirs.push(resourcesDir)
    const packagedMigrations = join(resourcesDir, 'db/migrations')
    const sourceMigrations = resolve(import.meta.dir, '../../../drizzle')
    await writeMigrationJournal(packagedMigrations, '0000_missing')

    expect(resolveMigrationsFolder(resourcesDir)).toBe(sourceMigrations)
  })

  it('falls back to source migrations when packaged resources are unavailable', async () => {
    const resourcesDir = await mkdtemp(join(tmpdir(), 'claw-resources-'))
    tempDirs.push(resourcesDir)
    const sourceMigrations = resolve(import.meta.dir, '../../../drizzle')

    expect(resolveMigrationsFolder(resourcesDir)).toBe(sourceMigrations)
  })

  it('reset drops the singleton; the next getAuditDb rebuilds a fresh DB', () => {
    const a = setAuditDbForTesting()
    a.insert(toolDispatches)
      .values({
        agentId: 'a',
        slug: 'a',
        agentLabel: 'a',
        sessionId: 's',
        toolName: 'navigate',
      })
      .run()
    expect(a.select().from(toolDispatches).all().length).toBe(1)
    resetAuditDbForTesting()
    const b = setAuditDbForTesting()
    expect(b.select().from(toolDispatches).all()).toEqual([])
  })
})

async function writeMigrationSet(
  migrationsFolder: string,
  tag: string,
): Promise<void> {
  await writeMigrationJournal(migrationsFolder, tag)
  await writeFile(join(migrationsFolder, `${tag}.sql`), 'SELECT 1;')
}

async function writeMigrationJournal(
  migrationsFolder: string,
  tag: string,
): Promise<void> {
  await mkdir(join(migrationsFolder, 'meta'), { recursive: true })
  await writeFile(
    join(migrationsFolder, 'meta', '_journal.json'),
    JSON.stringify({ entries: [{ tag }] }),
  )
}
