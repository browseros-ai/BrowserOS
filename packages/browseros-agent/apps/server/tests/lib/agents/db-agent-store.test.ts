/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtempSync } from 'node:fs'
import { rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { eq } from 'drizzle-orm'
import { DbAgentStore } from '../../../src/lib/agents/db-agent-store'
import { closeDb, initializeDb } from '../../../src/lib/db'
import { agentDefinitions } from '../../../src/lib/db/schema'

describe('DbAgentStore', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    closeDb()
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  it('creates, lists, loads, updates, and deletes named agents', async () => {
    const store = createStore()

    const agent = await store.create({
      name: ' Review bot ',
      adapter: 'codex',
      modelId: 'gpt-5.5',
      reasoningEffort: 'medium',
    })

    expect(agent).toMatchObject({
      name: 'Review bot',
      adapter: 'codex',
      modelId: 'gpt-5.5',
      reasoningEffort: 'medium',
      permissionMode: 'approve-all',
      sessionKey: `agent:${agent.id}:main`,
      pinned: false,
    })

    const updated = await store.update(agent.id, {
      name: 'Renamed bot',
      pinned: true,
    })

    expect(updated).toMatchObject({
      id: agent.id,
      name: 'Renamed bot',
      pinned: true,
    })
    expect(await store.get(agent.id)).toEqual(updated)
    expect(await store.list()).toEqual([updated])
    expect(await store.delete(agent.id)).toBe(true)
    expect(await store.delete(agent.id)).toBe(false)
    expect(await store.list()).toEqual([])
  })

  it('serializes concurrent creates without dropping agents', async () => {
    const store = createStore()

    const created = await Promise.all(
      Array.from({ length: 10 }, (_, index) =>
        store.create({
          name: `Agent ${index}`,
          adapter: index % 2 === 0 ? 'codex' : 'claude',
        }),
      ),
    )

    const listed = await store.list()
    expect(listed).toHaveLength(created.length)
    expect(new Set(listed.map((agent) => agent.id)).size).toBe(created.length)
  })

  it('persists OpenClaw adapter config with the agent record', async () => {
    const { db, store } = createStoreWithDb()

    const agent = await store.create({
      name: 'OpenClaw bot',
      adapter: 'openclaw',
      providerType: 'openai-compatible',
      providerName: 'Kimi',
      baseUrl: 'https://api.fireworks.ai/inference/v1',
      apiKey: 'test-key',
      supportsImages: true,
    })

    const row = db
      .select()
      .from(agentDefinitions)
      .where(eq(agentDefinitions.id, agent.id))
      .get()

    expect(JSON.parse(row?.adapterConfigJson ?? '{}')).toEqual({
      providerType: 'openai-compatible',
      providerName: 'Kimi',
      baseUrl: 'https://api.fireworks.ai/inference/v1',
      apiKey: 'test-key',
      supportsImages: true,
    })
  })

  it('upserts gateway-owned OpenClaw records idempotently', async () => {
    const store = createStore()

    const first = await store.upsertExisting({
      id: 'oc-existing',
      name: 'Gateway agent',
      adapter: 'openclaw',
      modelId: 'openrouter/anthropic/claude-sonnet-4.5',
    })
    const second = await store.upsertExisting({
      id: 'oc-existing',
      name: 'Changed gateway name',
      adapter: 'openclaw',
    })

    expect(second).toEqual(first)
    expect(await store.list()).toEqual([first])
  })

  it('roundtrips customCommand/customArgs/customLabel through serialize/deserialize', async () => {
    const { db, store } = createStoreWithDb()

    const agent = await store.create({
      name: 'my-custom-agent',
      adapter: 'custom',
      customCommand: './bin/my-acp-server',
      customArgs: ['acp', '--verbose'],
      customLabel: 'My Custom Agent 🚀',
    })

    // Verify the returned object has custom fields
    expect(agent).toMatchObject({
      name: 'my-custom-agent',
      adapter: 'custom',
      customCommand: './bin/my-acp-server',
      customArgs: ['acp', '--verbose'],
      customLabel: 'My Custom Agent 🚀',
    })

    // Verify persisted in adapterConfigJson
    const row = db
      .select()
      .from(agentDefinitions)
      .where(eq(agentDefinitions.id, agent.id))
      .get()
    expect(JSON.parse(row?.adapterConfigJson ?? '{}')).toMatchObject({
      customCommand: './bin/my-acp-server',
      customArgs: ['acp', '--verbose'],
      customLabel: 'My Custom Agent 🚀',
    })

    // Verify get() roundtrips
    const reloaded = await store.get(agent.id)
    expect(reloaded).toMatchObject({
      customCommand: './bin/my-acp-server',
      customArgs: ['acp', '--verbose'],
      customLabel: 'My Custom Agent 🚀',
    })
  })

  it('handles custom agent without optional custom fields', async () => {
    const store = createStore()

    const agent = await store.create({
      name: 'minimal-custom',
      adapter: 'custom',
      customCommand: 'gemini',
    })

    expect(agent).toMatchObject({
      customCommand: 'gemini',
    })
    expect(agent.customArgs).toBeUndefined()
    expect(agent.customLabel).toBeUndefined()
  })

  function createStore(): DbAgentStore {
    return createStoreWithDb().store
  }

  function createStoreWithDb() {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-db-agents-test-'))
    tempDirs.push(dir)
    const handle = initializeDb({
      dbPath: join(dir, 'db', 'browseros.sqlite'),
    })
    return { db: handle.db, store: new DbAgentStore({ db: handle.db }) }
  }
})
