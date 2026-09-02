import { afterEach, describe, expect, test } from 'bun:test'
import { mkdtempSync } from 'node:fs'
import { rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { closeDb, initializeDb } from '../../../src/lib/db'
import { dbLlmProviderStore } from '../../../src/lib/llm-providers/provider-store'

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

describe('dbLlmProviderStore', () => {
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

    const saved = await dbLlmProviderStore.insertIfAbsent(baseProvider())

    expect(saved?.id).toBe(PROVIDER_ID)
    expect(saved?.apiKey).toBe('sk-test')
    expect(await dbLlmProviderStore.list()).toHaveLength(1)
  })

  // The behaviour the whole import design rests on: onConflictDoNothing must
  // return no row, and must leave the existing one exactly as it was.
  test('insertIfAbsent returns null and changes nothing when the id exists', async () => {
    useTempDb()
    await dbLlmProviderStore.upsert({ ...baseProvider(), name: 'Edited since' })

    const saved = await dbLlmProviderStore.insertIfAbsent({
      ...baseProvider(),
      name: 'Stale copy',
      apiKey: 'sk-stale',
    })

    expect(saved).toBeNull()
    const existing = await dbLlmProviderStore.get(PROVIDER_ID)
    expect(existing?.name).toBe('Edited since')
    expect(existing?.apiKey).toBe('sk-test')
  })

  // Integer would floor this to 0 and silently make every model deterministic.
  test('temperature survives as a fraction', async () => {
    useTempDb()
    await dbLlmProviderStore.insertIfAbsent({
      ...baseProvider(),
      temperature: 0.2,
    })

    expect((await dbLlmProviderStore.get(PROVIDER_ID))?.temperature).toBe(0.2)
  })

  test('insertIfAbsent preserves the creation time it is given', async () => {
    useTempDb()
    await dbLlmProviderStore.insertIfAbsent({
      ...baseProvider(),
      createdAt: 42,
    })

    expect((await dbLlmProviderStore.get(PROVIDER_ID))?.createdAt).toBe(42)
  })
})
