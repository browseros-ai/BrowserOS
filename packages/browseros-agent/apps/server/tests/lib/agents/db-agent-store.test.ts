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
import { DbAgentStore } from '../../../src/lib/agents/storage/db-agent-store'
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

  it('does not persist adapter config for built-in agents', async () => {
    const { db, store } = createStoreWithDb()

    const agent = await store.create({
      name: 'Codex bot',
      adapter: 'codex',
      modelId: 'gpt-5.5',
    })

    const row = db
      .select()
      .from(agentDefinitions)
      .where(eq(agentDefinitions.id, agent.id))
      .get()

    expect(row?.adapterConfigJson).toBeNull()
  })

  it('upserts existing records idempotently', async () => {
    const store = createStore()

    const first = await store.upsertExisting({
      id: 'agent-existing',
      name: 'Imported agent',
      adapter: 'codex',
      modelId: 'openrouter/anthropic/claude-sonnet-4.5',
    })
    const second = await store.upsertExisting({
      id: 'agent-existing',
      name: 'Changed imported name',
      adapter: 'codex',
    })

    expect(second).toEqual(first)
    expect(await store.list()).toEqual([first])
  })

  it('ignores stale rows with unsupported adapter ids', async () => {
    const { db, store } = createStoreWithDb()
    db.insert(agentDefinitions)
      .values([
        {
          id: 'stale-agent',
          name: 'Stale agent',
          adapter: 'removed-adapter' as never,
          modelId: 'default',
          reasoningEffort: 'medium',
          permissionMode: 'approve-all',
          sessionKey: 'agent:stale-agent:main',
          pinned: false,
          adapterConfigJson: null,
          createdAt: 1000,
          updatedAt: 1000,
        },
        {
          id: 'legacy-hermes-agent',
          name: 'Legacy Hermes agent',
          adapter: 'hermes' as never,
          modelId: 'default',
          reasoningEffort: 'medium',
          permissionMode: 'approve-all',
          sessionKey: 'agent:legacy-hermes-agent:main',
          pinned: false,
          adapterConfigJson: null,
          createdAt: 1000,
          updatedAt: 1000,
        },
      ])
      .run()

    expect(await store.get('stale-agent')).toBeNull()
    expect(await store.get('legacy-hermes-agent')).toBeNull()
    expect(await store.list()).toEqual([])
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
