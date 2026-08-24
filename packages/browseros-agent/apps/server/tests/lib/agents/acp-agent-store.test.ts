import { Database } from 'bun:sqlite'
import { afterEach, describe, expect, test } from 'bun:test'
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  writeFileSync,
} from 'node:fs'
import { rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { drizzle } from 'drizzle-orm/bun-sqlite'
import { migrate } from 'drizzle-orm/bun-sqlite/migrator'
import {
  DbAcpAgentStore,
  deriveAcpSessionKey,
} from '../../../src/lib/agents/storage/acp-agent-store'
import { closeDb, initializeDb } from '../../../src/lib/db'

describe('DbAcpAgentStore', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    closeDb()
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  test('persists only ACP identity and optional agent choices', async () => {
    const store = createStore()
    const agent = await store.create({
      name: ' Review agent ',
      type: 'codex',
      modelId: ' gpt-5.5 ',
      reasoningEffort: ' high ',
      workingDirectory: ' /tmp/project ',
    })

    expect(agent).toMatchObject({
      name: 'Review agent',
      type: 'codex',
      modelId: 'gpt-5.5',
      reasoningEffort: 'high',
      workingDirectory: '/tmp/project',
    })
    expect('permissionMode' in agent).toBe(false)
    expect('sessionKey' in agent).toBe(false)
    expect('adapter' in agent).toBe(false)
    expect(await store.list()).toEqual([agent])
  })

  test('leaves model, reasoning, and working directory unset by default', async () => {
    const store = createStore()
    const agent = await store.create({ name: 'Claude', type: 'claude' })

    expect(agent.modelId).toBeUndefined()
    expect(agent.reasoningEffort).toBeUndefined()
    expect(agent.workingDirectory).toBeUndefined()
  })

  test('deletes the record', async () => {
    const store = createStore()
    const agent = await store.create({ name: 'Claude', type: 'claude' })

    expect(await store.delete(agent.id)).toBe(true)
    expect(await store.delete(agent.id)).toBe(false)
    expect(await store.get(agent.id)).toBeNull()
  })

  test('round-trips a custom agent config through the DB', async () => {
    const store = createStore()
    const customConfig = {
      command: 'npx -y @scope/my-agent-acp --stdio',
      env: { MY_AGENT_KEY: 'secret' },
      fullAccessModes: ['bypass'],
      reasoningEffortKey: 'effort',
      icon: '🤖',
    }
    const agent = await store.create({
      name: 'My Agent',
      type: 'custom',
      customConfig,
    })

    expect(agent.type).toBe('custom')
    expect(agent.customConfig).toEqual(customConfig)
    expect((await store.get(agent.id))?.customConfig).toEqual(customConfig)
    expect((await store.list())[0]?.customConfig).toEqual(customConfig)
  })

  test('updates name and custom config, and 404s an unknown id', async () => {
    const store = createStore()
    const agent = await store.create({
      name: 'A',
      type: 'custom',
      customConfig: { command: 'old' },
    })

    const updated = await store.update(agent.id, {
      name: 'B',
      customConfig: { command: 'new --stdio' },
    })
    expect(updated).toMatchObject({
      name: 'B',
      customConfig: { command: 'new --stdio' },
    })
    expect(updated?.updatedAt).toBeGreaterThanOrEqual(agent.updatedAt)
    expect(
      await store.update('00000000-0000-4000-8000-0000000000ff', { name: 'x' }),
    ).toBeNull()
  })

  test('migration resets legacy harness state without clearing unrelated data', () => {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-acp-migration-test-'))
    tempDirs.push(dir)
    const dbPath = join(dir, 'browseros.sqlite')
    const sourceMigrations = resolve(
      import.meta.dir,
      '../../../src/lib/db/migrations',
    )
    const legacyMigrations = join(dir, 'legacy-migrations')
    mkdirSync(join(legacyMigrations, 'meta'), { recursive: true })

    const journal = JSON.parse(
      readFileSync(join(sourceMigrations, 'meta', '_journal.json'), 'utf8'),
    ) as { version: string; dialect: string; entries: unknown[] }
    writeFileSync(
      join(legacyMigrations, 'meta', '_journal.json'),
      JSON.stringify({ ...journal, entries: journal.entries.slice(0, 4) }),
    )
    for (const tag of [
      '0000_zippy_psylocke',
      '0001_lazy_orphan',
      '0002_chemical_whirlwind',
      '0003_scrub_hermes_credentials',
    ]) {
      copyFileSync(
        join(sourceMigrations, `${tag}.sql`),
        join(legacyMigrations, `${tag}.sql`),
      )
    }

    const legacySqlite = new Database(dbPath)
    migrate(drizzle(legacySqlite), { migrationsFolder: legacyMigrations })
    legacySqlite.run(`
      INSERT INTO agent_definitions (
        id, name, adapter, model_id, reasoning_effort, permission_mode,
        session_key, pinned, adapter_config_json, created_at, updated_at
      ) VALUES (
        'legacy-agent', 'Legacy', 'codex', 'default', 'medium', 'approve-all',
        'agent:legacy-agent:main', 0, NULL, 1, 1
      )
    `)
    legacySqlite.run(`
      INSERT INTO produced_files (
        id, agent_definition_id, session_key, turn_id, turn_prompt, path,
        size, mtime_ms, created_at, detected_by
      ) VALUES (
        'file-1', 'legacy-agent', 'agent:legacy-agent:main', 'turn-1', 'prompt',
        '/tmp/file', 1, 1, 1, 'diff'
      )
    `)
    legacySqlite.run(`
      INSERT INTO oauth_tokens (
        browseros_id, provider, access_token, refresh_token, expires_at,
        email, account_id, updated_at
      ) VALUES ('browseros-1', 'browseros', 'access', 'refresh', 1, NULL, NULL, 1)
    `)
    legacySqlite.close()

    const handle = initializeDb({ dbPath, migrationsDir: sourceMigrations })
    expect(
      handle.sqlite
        .query(
          "SELECT COUNT(*) AS count FROM sqlite_master WHERE type = 'table' AND name = 'agent_definitions'",
        )
        .get(),
    ).toEqual({ count: 0 })
    expect(
      handle.sqlite
        .query(
          "SELECT COUNT(*) AS count FROM sqlite_master WHERE type = 'table' AND name = 'produced_files'",
        )
        .get(),
    ).toEqual({ count: 0 })
    expect(
      handle.sqlite.query('SELECT COUNT(*) AS count FROM acp_agents').get(),
    ).toEqual({ count: 0 })
    expect(
      handle.sqlite
        .query('PRAGMA table_info(acp_agents)')
        .all()
        .some((column) => (column as { name: string }).name === 'pinned'),
    ).toBe(false)
    expect(
      handle.sqlite.query('SELECT COUNT(*) AS count FROM oauth_tokens').get(),
    ).toEqual({ count: 1 })
  })

  function createStore(): DbAcpAgentStore {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-acp-agents-test-'))
    tempDirs.push(dir)
    const handle = initializeDb({
      dbPath: join(dir, 'db', 'browseros.sqlite'),
    })
    return new DbAcpAgentStore({ db: handle.db })
  }
})

describe('deriveAcpSessionKey', () => {
  test('is stable per agent and conversation without persisted state', () => {
    expect(deriveAcpSessionKey('agent-1', 'conversation-1')).toBe(
      'acp:agent-1:conversation-1',
    )
    expect(deriveAcpSessionKey('agent-1', 'conversation-2')).not.toBe(
      deriveAcpSessionKey('agent-1', 'conversation-1'),
    )
  })
})
