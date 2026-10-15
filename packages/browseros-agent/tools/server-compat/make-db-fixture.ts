/**
 * TS side of the server-rust database compatibility proof.
 *
 * `create` writes SQLite fixtures with the TS server's own drizzle migrations and stores, plus
 * what those stores read back (`expected.json`), into apps/server-rust/tests/fixtures/ts-db.
 * `dump` opens any file the way the TS server boots (initializeDb, which migrates) and writes
 * what every store reads, so the Rust tests can prove a Rust-touched file reads identically
 * here. Dumps go to a file, never stdout, because they contain credentials.
 *
 *   bun tools/server-compat/make-db-fixture.ts create [outDir]
 *   bun tools/server-compat/make-db-fixture.ts dump <dbPath> <outJson>
 */
import type { Database as BunDatabase } from 'bun:sqlite'
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { DbAcpAgentStore } from '../../apps/server/src/lib/agents/storage/acp-agent-store'
import { OAuthTokenStore } from '../../apps/server/src/lib/clients/oauth/token-store'
import { DbConversationStore } from '../../apps/server/src/lib/conversations/conversation-store'
import {
  closeDb,
  getDbHandle,
  initializeDb,
} from '../../apps/server/src/lib/db'
import { dbProviderStore } from '../../apps/server/src/lib/providers/provider-store'
import { dbScheduledJobRunStore } from '../../apps/server/src/lib/schedules/run-store'
import { dbScheduledJobStore } from '../../apps/server/src/lib/schedules/schedule-store'

const ROOT = resolve(import.meta.dir, '../..')
const SERVER_DIR = join(ROOT, 'apps/server')
const MIGRATIONS_DIR = join(SERVER_DIR, 'src/lib/db/migrations')
const DEFAULT_OUT = join(ROOT, 'apps/server-rust/tests/fixtures/ts-db')
const INSTALL_ID = 'install-fixture'

/** Everything the TS stores read from one file; the Rust tests build the same shape. */
async function dump(dbPath: string, outJson: string): Promise<void> {
  const { sqlite } = initializeDb({ dbPath })
  const conversations = new DbConversationStore()
  const tokens = new OAuthTokenStore(getDbHandle().db)
  const summaries = await conversations.list()
  const publicProviders = await dbProviderStore.list()
  const tokenKeys = sqlite
    .query<{ browseros_id: string; provider: string }, []>(
      'SELECT browseros_id, provider FROM oauth_tokens ORDER BY browseros_id, provider',
    )
    .all()
  const result = {
    drizzleMigrations: sqlite
      .query(
        'SELECT hash, created_at FROM __drizzle_migrations ORDER BY created_at',
      )
      .all(),
    schema: schemaOf(sqlite),
    providers: await Promise.all(
      publicProviders.map((row) => dbProviderStore.getWithCredentials(row.id)),
    ),
    publicProviders,
    defaultProviderId: (await dbProviderStore.getDefault())?.id ?? null,
    agents: await new DbAcpAgentStore().list(),
    conversationSummaries: summaries,
    conversations: await Promise.all(
      summaries.map((summary) => conversations.get(summary.id)),
    ),
    jobs: await dbScheduledJobStore.list(),
    runs: await dbScheduledJobRunStore.list(),
    oauthTokens: tokenKeys.map((key) => ({
      browserosId: key.browseros_id,
      provider: key.provider,
      tokens: tokens.getTokens(key.browseros_id, key.provider),
      status: tokens.getStatus(key.browseros_id, key.provider),
    })),
  }
  closeDb()
  writeFileSync(outJson, `${JSON.stringify(result, null, 2)}\n`)
}

function schemaOf(sqlite: BunDatabase) {
  return sqlite
    .query(
      "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' ORDER BY type, name",
    )
    .all()
}

/** A current-schema file written only through the TS stores, with every column exercised. */
async function writeCurrent(dbPath: string): Promise<void> {
  initializeDb({ dbPath })
  await dbProviderStore.upsert({
    id: 'openai-main',
    type: 'openai',
    name: 'OpenAI',
    modelId: 'gpt-5',
    contextWindow: 128000,
    supportsImages: false,
    temperature: 0.7,
    apiKey: 'sk-fixture-openai',
    headers: { 'X-Zeta': 'z', 'X-Alpha': 'a' },
    reasoningEffort: 'high',
    reasoningSummary: 'auto',
  })
  await dbProviderStore.upsert({
    id: 'anthropic-1',
    type: 'anthropic',
    name: 'Claude',
    modelId: 'claude-opus',
    contextWindow: 200000,
    apiKey: 'sk-ant-fixture',
    createdAt: 1_700_000_000_000,
  })
  await dbProviderStore.upsert({
    id: 'bedrock-1',
    type: 'bedrock',
    name: 'Bedrock',
    modelId: 'anthropic.claude',
    contextWindow: 100000,
    accessKeyId: 'AKIAFIXTURE',
    secretAccessKey: 'secret-fixture',
    sessionToken: 'session-fixture',
    region: 'us-west-2',
  })
  await dbProviderStore.upsert({
    id: 'azure-1',
    type: 'azure',
    name: 'Azure',
    modelId: 'gpt-4o',
    contextWindow: 64000,
    resourceName: 'my-resource',
    apiKey: '',
    baseUrl: null,
  })
  await dbProviderStore.upsert({
    id: 'ollama-local',
    type: 'ollama',
    name: 'Ollama',
    modelId: 'llama3',
    contextWindow: 8192,
    baseUrl: 'http://localhost:11434',
  })
  await dbProviderStore.setDefault('anthropic-1')

  const agents = new DbAcpAgentStore()
  const claude = await agents.create({
    name: '  Claude Code  ',
    type: 'claude',
    modelId: 'opus',
    reasoningEffort: ' high ',
    workingDirectory: '  /tmp/work dir  ',
  })
  await agents.create({ name: 'Codex', type: 'codex' })
  await agents.create({
    name: 'Mine',
    type: 'custom',
    customConfig: {
      command: 'my-agent --acp',
      env: { TOKEN: 'fixture' },
      fullAccessModes: ['yolo'],
      icon: 'bot',
    },
  })

  const conversations = new DbConversationStore()
  await conversations.save({
    id: '11111111-1111-4111-8111-111111111111',
    targetType: 'browseros',
    origin: 'sidepanel',
    messages: [
      {
        id: 'u1',
        role: 'user',
        parts: [{ type: 'text', text: 'Open the docs page' }],
      },
      {
        id: 'a1',
        role: 'assistant',
        metadata: { model: 'gpt-5' },
        parts: [
          { type: 'step-start' },
          { type: 'reasoning', text: 'thinking', state: 'done' },
          {
            type: 'dynamic-tool',
            toolName: 'navigate',
            toolCallId: 'call-1',
            state: 'output-available',
            input: { url: 'https://example.com' },
            output: null,
            callProviderMetadata: { openai: { itemId: 'x' } },
          },
          {
            type: 'tool-suggest_schedule',
            toolCallId: 'call-2',
            state: 'output-error',
            input: { query: 'daily' },
            errorText: 'nope',
            rawInput: { legacy: true },
          },
          { type: 'data-progress', id: 'p1', data: { pct: 50 } },
          { type: 'text', text: 'Done.', state: 'done', extraKey: 1 },
        ],
      },
    ] as never,
  })
  await conversations.save({
    id: '22222222-2222-4222-8222-222222222222',
    targetType: 'claude',
    agentId: claude.id,
    messages: [
      {
        id: 'u2',
        role: 'user',
        parts: [
          { type: 'file', mediaType: 'image/png', url: 'data:,' },
          { type: 'text', text: `  ${'é'.repeat(230)}  ` },
        ],
      },
    ] as never,
  })
  await conversations.insertIfAbsent({
    id: '33333333-3333-4333-8333-333333333333',
    targetType: 'codex',
    lastMessagedAt: 1_600_000_000_000,
    // Imports are unvalidated: a legacy user message with extra keys, then a non-UIMessage.
    messages: [
      {
        legacy: true,
        role: 'user',
        parts: [{ type: 'text', text: 'legacy question' }],
      },
      { role: 'assistant', content: 'unstructured' },
    ] as never,
  })

  await dbScheduledJobStore.upsert({
    id: 'job-daily',
    name: 'Daily digest',
    query: 'summarize',
    scheduleType: 'daily',
    scheduleTime: '09:30',
    providerId: 'openai-main',
  })
  await dbScheduledJobStore.upsert({
    id: 'job-hourly',
    name: 'Hourly',
    query: 'check',
    scheduleType: 'hourly',
    scheduleInterval: 2,
    enabled: false,
    lastRunAt: 1_750_000_000_000,
  })
  await dbScheduledJobStore.upsert({
    id: 'job-minutes',
    name: 'Minutes',
    query: 'poll',
    scheduleType: 'minutes',
    scheduleInterval: 15,
    providerId: null,
  })
  // 20 runs through the store's import-style path (no prune), so the file holds more than the
  // 15 a route write keeps and a Rust run upsert has something to prune.
  for (let index = 0; index < 20; index++) {
    await dbScheduledJobRunStore.upsert({
      id: `run-${String(index).padStart(2, '0')}`,
      jobId: 'job-daily',
      status: index === 19 ? 'running' : 'completed',
      startedAt: 1_760_000_000_000 + index * 60_000,
      completedAt: index === 19 ? null : 1_760_000_030_000 + index * 60_000,
      result: `result ${index}`,
      toolCalls:
        index % 5 === 0
          ? [
              {
                id: `tc-${index}`,
                name: 'navigate',
                input: { url: 'https://example.com' },
                output: { ok: true },
                timestamp: '2025-10-14T00:00:00.000Z',
              },
            ]
          : null,
    })
  }
  await dbScheduledJobRunStore.upsert({
    id: 'run-hourly',
    jobId: 'job-hourly',
    status: 'failed',
    startedAt: 1_760_100_000_000,
    error: 'boom',
    executionLog: 'log',
    finalResult: 'final',
  })

  const tokens = new OAuthTokenStore(getDbHandle().db)
  tokens.upsertTokens(INSTALL_ID, 'chatgpt-pro', {
    accessToken: 'access-fixture',
    refreshToken: 'refresh-fixture',
    expiresAt: 1_900_000_000_000,
    email: 'user@example.com',
    accountId: 'acct-1',
  })
  tokens.upsertTokens(INSTALL_ID, 'github-copilot', {
    accessToken: 'gho-fixture',
    refreshToken: 'gho-refresh',
    expiresAt: 1_900_000_000_000,
  })
  checkpointAndClose()
}

/** A file stopped at migration 0010 with data written at 0009: llm_providers, acp_agents, and
 * scheduled_jobs still pointing at llm_providers, plus 20 runs that 0011 must not lose. */
async function writeV0010(dbPath: string): Promise<void> {
  await migrateTo(dbPath, 9)
  const { Database } = await import('bun:sqlite')
  const sqlite = new Database(dbPath)
  sqlite.exec('PRAGMA foreign_keys = ON')
  const at = 1_760_000_000_000
  const stamps = { created_at: at, updated_at: at }
  const provider = { supports_images: 1, temperature: 0.2, ...stamps }
  insertRows(sqlite, 'llm_providers', [
    {
      id: 'openai-main',
      type: 'openai',
      name: 'OpenAI',
      model_id: 'gpt-5',
      context_window: 128000,
      api_key: 'sk-legacy',
      ...provider,
    },
    {
      id: 'browseros',
      type: 'browseros',
      name: 'BrowserOS',
      model_id: 'auto',
      context_window: 200000,
      ...provider,
    },
    {
      id: 'groq-1',
      type: 'groq',
      name: 'Groq',
      base_url: 'https://api.groq.com',
      model_id: 'llama',
      context_window: 32000,
      ...provider,
      supports_images: 0,
      temperature: 0.5,
      api_key: 'gsk-legacy',
    },
  ])
  insertRows(sqlite, 'acp_agents', [
    {
      id: '44444444-4444-4444-8444-444444444444',
      name: 'Claude',
      type: 'claude',
      model_id: 'sonnet',
      working_directory: '/tmp',
      ...stamps,
      updated_at: at + 1,
    },
    {
      id: '55555555-5555-4555-8555-555555555555',
      name: 'Custom',
      type: 'custom',
      custom_config: '{"command":"agent"}',
      ...stamps,
      updated_at: at + 2,
    },
  ])
  insertRows(sqlite, 'scheduled_jobs', [
    {
      id: 'job-a',
      name: 'A',
      query: 'q',
      schedule_type: 'daily',
      schedule_time: '08:00',
      enabled: 1,
      provider_id: 'openai-main',
      ...stamps,
    },
    {
      id: 'job-b',
      name: 'B',
      query: 'q',
      schedule_type: 'hourly',
      schedule_interval: 3,
      enabled: 1,
      provider_id: 'browseros',
      ...stamps,
    },
    {
      id: 'job-c',
      name: 'C',
      query: 'q',
      schedule_type: 'minutes',
      schedule_interval: 30,
      enabled: 0,
      last_run_at: at,
      ...stamps,
    },
  ])
  insertRows(
    sqlite,
    'scheduled_job_runs',
    Array.from({ length: 20 }, (_, index) => ({
      id: `legacy-run-${index}`,
      job_id: index < 17 ? 'job-a' : 'job-b',
      status: 'completed',
      started_at: at + index,
      completed_at: at + index + 5,
      result: `r${index}`,
      ...stamps,
    })),
  )
  insertRows(sqlite, 'conversations', [
    {
      id: '66666666-6666-4666-8666-666666666666',
      messages: JSON.stringify([
        { id: 'u', role: 'user', parts: [{ type: 'text', text: 'hi' }] },
      ]),
      last_user_message: 'hi',
      target_type: 'browseros',
      last_messaged_at: at,
      ...stamps,
    },
  ])
  insertRows(sqlite, 'oauth_tokens', [
    {
      browseros_id: INSTALL_ID,
      provider: 'qwen-code',
      access_token: 'qa',
      refresh_token: 'qr',
      expires_at: at,
      updated_at: at,
    },
  ])
  sqlite.close()
  await migrateTo(dbPath, 10)
  const final = new Database(dbPath)
  final.exec('PRAGMA wal_checkpoint(TRUNCATE)')
  final.close()
}

function insertRows(
  sqlite: BunDatabase,
  table: string,
  rows: Record<string, string | number>[],
): void {
  for (const row of rows) {
    const columns = Object.keys(row)
    sqlite
      .prepare(
        `INSERT INTO ${table} (${columns.join(', ')}) VALUES (${columns.map(() => '?').join(', ')})`,
      )
      .run(...Object.values(row))
  }
}

/** drizzle's own migrate() over the first `last + 1` journal entries (a TS build of that era). */
async function migrateTo(dbPath: string, last: number): Promise<void> {
  const folder = mkdtempSync(join(tmpdir(), 'browseros-migrations-'))
  const journal = JSON.parse(
    readFileSync(join(MIGRATIONS_DIR, 'meta/_journal.json'), 'utf8'),
  ) as { entries: { tag: string }[] }
  journal.entries = journal.entries.slice(0, last + 1)
  mkdirSync(join(folder, 'meta'))
  writeFileSync(join(folder, 'meta/_journal.json'), JSON.stringify(journal))
  for (const entry of journal.entries) {
    copyFileSync(
      join(MIGRATIONS_DIR, `${entry.tag}.sql`),
      join(folder, `${entry.tag}.sql`),
    )
  }
  const { Database } = await import('bun:sqlite')
  const { drizzle } = await import(
    Bun.resolveSync('drizzle-orm/bun-sqlite', SERVER_DIR)
  )
  const { migrate } = await import(
    Bun.resolveSync('drizzle-orm/bun-sqlite/migrator', SERVER_DIR)
  )
  const sqlite = new Database(dbPath)
  sqlite.exec('PRAGMA journal_mode = WAL')
  sqlite.exec('PRAGMA foreign_keys = ON')
  migrate(drizzle(sqlite), { migrationsFolder: folder })
  sqlite.close()
  rmSync(folder, { recursive: true, force: true })
}

function checkpointAndClose(): void {
  getDbHandle().sqlite.exec('PRAGMA wal_checkpoint(TRUNCATE)')
  closeDb()
}

async function create(outDir: string): Promise<void> {
  const scratch = mkdtempSync(join(tmpdir(), 'browseros-db-fixture-'))
  const fresh = join(scratch, 'fresh.sqlite')
  initializeDb({ dbPath: fresh })
  writeJson(join(outDir, 'fresh/schema.json'), {
    schema: schemaOf(getDbHandle().sqlite),
    drizzleMigrations: getDbHandle()
      .sqlite.query(
        'SELECT hash, created_at FROM __drizzle_migrations ORDER BY created_at',
      )
      .all(),
  })
  closeDb()

  const v0014 = join(outDir, 'v0014/browseros.sqlite')
  mkdirSync(dirname(v0014), { recursive: true })
  rmSync(v0014, { force: true })
  await writeCurrent(v0014)
  const v0014Copy = join(scratch, 'v0014.sqlite')
  copyFileSync(v0014, v0014Copy)
  await dump(v0014Copy, join(outDir, 'v0014/expected.json'))

  const v0010 = join(outDir, 'v0010/browseros.sqlite')
  mkdirSync(dirname(v0010), { recursive: true })
  rmSync(v0010, { force: true })
  await writeV0010(v0010)
  // What the TS server itself makes of this file: same schema, but 0011's DROP TABLE runs
  // with foreign keys still on (the pragma is a no-op inside drizzle's transaction), so the
  // runs cascade away. The Rust tests compare against this, runs excepted.
  const v0010Copy = join(scratch, 'v0010.sqlite')
  copyFileSync(v0010, v0010Copy)
  await dump(v0010Copy, join(outDir, 'v0010/ts-catchup.json'))
  rmSync(scratch, { recursive: true, force: true })
  // Checkpointed and closed above, so these are empty leftovers; fixtures are single files.
  for (const db of [v0014, v0010]) {
    rmSync(`${db}-wal`, { force: true })
    rmSync(`${db}-shm`, { force: true })
  }
}

function writeJson(path: string, value: unknown): void {
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`)
}

const [command, ...args] = process.argv.slice(2)
if (command === 'create') {
  await create(resolve(args[0] ?? DEFAULT_OUT))
} else if (command === 'dump' && args.length === 2) {
  await dump(resolve(args[0]), resolve(args[1]))
} else {
  console.error(
    'usage: make-db-fixture.ts create [outDir] | dump <dbPath> <outJson>',
  )
  process.exit(2)
}
