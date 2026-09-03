import { afterEach, describe, expect, test } from 'bun:test'
import { mkdtempSync } from 'node:fs'
import { rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { closeDb, getDb, initializeDb } from '../../../src/lib/db'
import { providers } from '../../../src/lib/db/schema'
import { dbProviderStore } from '../../../src/lib/providers/provider-store'

const PROVIDER_ID = 'provider-1'

function baseProvider() {
  return {
    id: PROVIDER_ID,
    type: 'openai',
    name: 'My OpenAI',
    modelId: 'gpt-5.5',
    contextWindow: 200000,
    apiKey: 'sk-test',
  }
}

describe('dbProviderStore', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    closeDb()
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  function useTempDb() {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-providers-test-'))
    tempDirs.push(dir)
    initializeDb({ dbPath: join(dir, 'db', 'browseros.sqlite') })
  }

  test('insertIfAbsent writes a provider that is not there yet', async () => {
    useTempDb()

    const saved = await dbProviderStore.insertIfAbsent(baseProvider())

    expect(saved?.id).toBe(PROVIDER_ID)
    expect(saved?.apiKey).toBe('sk-test')
    expect(await dbProviderStore.list()).toHaveLength(1)
  })

  // The behaviour the whole import design rests on: onConflictDoNothing must
  // return no row, and must leave the existing one exactly as it was.
  test('insertIfAbsent returns null and changes nothing when the id exists', async () => {
    useTempDb()
    await dbProviderStore.upsert({ ...baseProvider(), name: 'Edited since' })

    const saved = await dbProviderStore.insertIfAbsent({
      ...baseProvider(),
      name: 'Stale copy',
      apiKey: 'sk-stale',
    })

    expect(saved).toBeNull()
    const existing = await dbProviderStore.get(PROVIDER_ID)
    expect(existing?.name).toBe('Edited since')
    expect(existing?.apiKey).toBe('sk-test')
  })

  // Integer would floor this to 0 and silently make every model deterministic.
  test('temperature survives as a fraction', async () => {
    useTempDb()
    await dbProviderStore.insertIfAbsent({
      ...baseProvider(),
      temperature: 0.2,
    })

    expect((await dbProviderStore.get(PROVIDER_ID))?.temperature).toBe(0.2)
  })

  test('insertIfAbsent preserves the creation time it is given', async () => {
    useTempDb()
    await dbProviderStore.insertIfAbsent({
      ...baseProvider(),
      createdAt: 42,
    })

    expect((await dbProviderStore.get(PROVIDER_ID))?.createdAt).toBe(42)
  })

  // The whole point of merging the tables: one default, of either kind.
  test('the default can be an acp agent, not only an llm provider', async () => {
    useTempDb()
    await dbProviderStore.upsert(baseProvider())
    getDb()
      .insert(providers)
      .values({
        id: 'acp-1',
        kind: 'acp',
        type: 'claude',
        name: 'Claude Code',
        createdAt: 1,
        updatedAt: 1,
      })
      .run()

    expect(await dbProviderStore.setDefault('acp-1')).toBe(true)

    expect(await dbProviderStore.getDefault()).toMatchObject({
      id: 'acp-1',
      kind: 'acp',
    })
  })

  // A partial unique index allows one default row, so moving it has to clear
  // the old one first or the write violates the index.
  test('setting a default clears the previous one', async () => {
    useTempDb()
    await dbProviderStore.upsert(baseProvider())
    await dbProviderStore.upsert({ ...baseProvider(), id: 'provider-2' })

    await dbProviderStore.setDefault(PROVIDER_ID)
    await dbProviderStore.setDefault('provider-2')

    expect((await dbProviderStore.getDefault())?.id).toBe('provider-2')
    expect(
      (await dbProviderStore.list()).filter((row) => row.isDefault),
    ).toHaveLength(1)
  })

  test('an unknown id is refused rather than stored as a stale pointer', async () => {
    useTempDb()
    expect(await dbProviderStore.setDefault('nope')).toBe(false)
    expect(await dbProviderStore.getDefault()).toBeNull()
  })

  test('deleting the default leaves no default behind', async () => {
    useTempDb()
    await dbProviderStore.upsert(baseProvider())
    await dbProviderStore.setDefault(PROVIDER_ID)

    await dbProviderStore.remove(PROVIDER_ID)

    expect(await dbProviderStore.getDefault()).toBeNull()
  })

  // A save must not move the selection as a side effect.
  test('upserting the default provider keeps it default', async () => {
    useTempDb()
    await dbProviderStore.upsert(baseProvider())
    await dbProviderStore.setDefault(PROVIDER_ID)

    await dbProviderStore.upsert({ ...baseProvider(), name: 'Renamed' })

    expect(await dbProviderStore.getDefault()).toMatchObject({
      id: PROVIDER_ID,
      name: 'Renamed',
    })
  })

  test('listLlm excludes acp agents', async () => {
    useTempDb()
    await dbProviderStore.upsert(baseProvider())
    getDb()
      .insert(providers)
      .values({
        id: 'acp-1',
        kind: 'acp',
        type: 'claude',
        name: 'Claude Code',
        createdAt: 1,
        updatedAt: 1,
      })
      .run()

    expect((await dbProviderStore.listLlm()).map((r) => r.id)).toEqual([
      PROVIDER_ID,
    ])
    expect(await dbProviderStore.list()).toHaveLength(2)
  })
})
