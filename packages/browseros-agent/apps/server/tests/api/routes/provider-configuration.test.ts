import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { LLMConfig } from '@browseros/shared/schemas/llm'
import { createProviderRoutes } from '../../../src/api/routes/provider'
import { createProvidersRoutes } from '../../../src/api/routes/providers'
import { closeDb, getDb, initializeDb } from '../../../src/lib/db'
import { providers, scheduledJobs } from '../../../src/lib/db/schema'
import { dbProviderStore } from '../../../src/lib/providers/provider-store'

const DEFAULT_URL = 'https://openrouter.ai/api/v1'
const KEY = 'diagnostic-key'
const config = {
  type: 'openrouter',
  name: 'OpenRouter',
  modelId: 'test-model',
  contextWindow: 128000,
  apiKey: KEY,
}

describe('server-owned provider configuration', () => {
  let dir: string
  let captured: LLMConfig[]
  let crud: ReturnType<typeof createProvidersRoutes>
  let testRoute: ReturnType<typeof createProviderRoutes>

  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), 'provider-configuration-'))
    initializeDb({ dbPath: join(dir, 'test.sqlite') })
    captured = []
    crud = createProvidersRoutes()
    testRoute = createProviderRoutes({
      testConnection: async (resolved) => {
        captured.push(resolved)
        return { success: true, message: 'ok' }
      },
    })
  })

  afterEach(() => {
    closeDb()
    rmSync(dir, { recursive: true, force: true })
  })

  const json = (body: unknown) => ({
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })

  it('saves effective defaults, redacts write responses, and tests using only the ID', async () => {
    const saved = await crud.request('/provider', {
      method: 'PUT',
      ...json(config),
    })
    expect(saved.status).toBe(200)
    const savedPublic = (await saved.json()).provider
    expect(savedPublic).not.toHaveProperty('apiKey')
    expect(savedPublic).toMatchObject({
      id: 'provider',
      baseUrl: DEFAULT_URL,
      hasApiKey: true,
    })
    const { provider } = await (await crud.request('/provider')).json()
    expect(provider).not.toHaveProperty('apiKey')
    const result = await testRoute.request('/', {
      method: 'POST',
      ...json({ providerId: provider.id }),
    })
    expect(result.status).toBe(200)
    expect(captured).toEqual([
      expect.objectContaining({
        baseUrl: DEFAULT_URL,
        apiKey: KEY,
        model: config.modelId,
      }),
    ])
  })

  it('rejects saving a new key-based provider without a key', async () => {
    const response = await crud.request('/provider', {
      method: 'PUT',
      ...json({ ...config, apiKey: '' }),
    })
    expect(response.status).toBe(400)
    expect((await response.json()).fieldErrors.apiKey).toContain('API key')
    expect(await dbProviderStore.get('provider')).toBeNull()
  })

  it('lets users repair malformed legacy URLs with replacement credentials', async () => {
    getDb()
      .insert(providers)
      .values({
        ...config,
        id: 'legacy',
        kind: 'llm',
        baseUrl: 'https://:443/v1',
        createdAt: 1,
        updatedAt: 1,
      })
      .run()
    const draft = {
      providerId: 'legacy',
      provider: config.type,
      model: config.modelId,
      baseUrl: DEFAULT_URL,
    }
    const missingKey = await testRoute.request('/', {
      method: 'POST',
      ...json(draft),
    })
    expect(missingKey.status).toBe(400)
    expect((await missingKey.json()).fieldErrors.apiKey).toContain(
      'destination changed',
    )
    expect(captured).toHaveLength(0)
    const repairedTest = await testRoute.request('/', {
      method: 'POST',
      ...json({ ...draft, apiKey: 'replacement' }),
    })
    expect(repairedTest.status).toBe(200)
    expect(captured[0]).toMatchObject({
      baseUrl: DEFAULT_URL,
      apiKey: 'replacement',
    })
    const repairedSave = await crud.request('/legacy', {
      method: 'PUT',
      ...json({ ...config, baseUrl: DEFAULT_URL, apiKey: 'replacement' }),
    })
    expect(repairedSave.status).toBe(200)
    expect(await dbProviderStore.getWithCredentials('legacy')).toMatchObject({
      baseUrl: DEFAULT_URL,
      apiKey: 'replacement',
    })
  })

  it('returns a model field error when testing an incomplete draft', async () => {
    const result = await testRoute.request('/', {
      method: 'POST',
      ...json({ provider: config.type, model: '', apiKey: KEY }),
    })
    expect(result.status).toBe(400)
    expect((await result.json()).fieldErrors).toEqual({
      modelId: 'Enter a model ID.',
    })
    expect(captured).toHaveLength(0)
  })

  it('keeps a stored key through a blank-key edit and concurrent saved tests', async () => {
    await crud.request('/provider', { method: 'PUT', ...json(config) })
    const edit = await crud.request('/provider', {
      method: 'PUT',
      ...json({ ...config, name: 'Renamed', apiKey: '' }),
    })
    expect(edit.status).toBe(200)
    const results = await Promise.all(
      Array.from({ length: 3 }, () =>
        testRoute.request('/', {
          method: 'POST',
          ...json({ providerId: 'provider' }),
        }),
      ),
    )
    expect(results.map((r) => r.status)).toEqual([200, 200, 200])
    expect(captured.every((c) => c.apiKey === KEY)).toBe(true)
  })

  it('backfills missing legacy URLs without touching custom URLs or keys', async () => {
    for (const [id, baseUrl] of [
      ['legacy', null],
      ['custom', 'https://proxy.example/v1'],
    ] as const) {
      getDb()
        .insert(providers)
        .values({
          ...config,
          id,
          kind: 'llm',
          baseUrl,
          createdAt: 1,
          updatedAt: 1,
        })
        .run()
    }
    closeDb()
    initializeDb({ dbPath: join(dir, 'test.sqlite') })
    expect(await dbProviderStore.getWithCredentials('legacy')).toMatchObject({
      baseUrl: DEFAULT_URL,
      apiKey: KEY,
    })
    expect(await dbProviderStore.getWithCredentials('custom')).toMatchObject({
      baseUrl: 'https://proxy.example/v1',
      apiKey: KEY,
    })
  })

  it('reuses a legacy key for an equivalent explicit default in an unsaved draft', async () => {
    getDb()
      .insert(providers)
      .values({
        ...config,
        id: 'legacy',
        kind: 'llm',
        createdAt: 1,
        updatedAt: 1,
      })
      .run()
    const response = await testRoute.request('/', {
      method: 'POST',
      ...json({
        providerId: 'legacy',
        provider: 'openrouter',
        model: 'draft-model',
        baseUrl: DEFAULT_URL,
        apiKey: '',
      }),
    })
    expect(response.status).toBe(200)
    expect(captured[0]).toMatchObject({
      apiKey: KEY,
      model: 'draft-model',
      baseUrl: DEFAULT_URL,
    })
    expect((await dbProviderStore.get('legacy'))?.modelId).toBe(config.modelId)
  })

  it('refuses both save and draft test when a changed destination tries to reuse the key', async () => {
    await crud.request('/provider', { method: 'PUT', ...json(config) })
    const baseUrl = 'https://different.example/v1'
    const save = await crud.request('/provider', {
      method: 'PUT',
      ...json({ ...config, baseUrl, apiKey: '' }),
    })
    expect(save.status).toBe(400)
    const draft = await testRoute.request('/', {
      method: 'POST',
      ...json({
        providerId: 'provider',
        provider: config.type,
        model: config.modelId,
        baseUrl,
        apiKey: '',
      }),
    })
    expect(draft.status).toBe(400)
    expect((await draft.json()).message).toContain('destination')
    expect(captured).toHaveLength(0)
    expect(
      (await dbProviderStore.getWithCredentials('provider'))?.baseUrl,
    ).toBe(DEFAULT_URL)
  })

  it('keeps supported local endpoints credentialless', async () => {
    const response = await crud.request('/local', {
      method: 'PUT',
      ...json({
        ...config,
        type: 'openai-compatible',
        baseUrl: 'http://localhost:1234/v1',
        apiKey: '',
      }),
    })
    expect(response.status).toBe(200)
  })

  it('returns the original OAuth provider ID on reconnect', async () => {
    const oauth = { ...config, type: 'chatgpt-pro', apiKey: '' }
    await crud.request('/original', { method: 'PUT', ...json(oauth) })
    await dbProviderStore.setDefault('original')
    const response = await crud.request('/reconnected', {
      method: 'PUT',
      ...json(oauth),
    })
    expect(response.status).toBe(200)
    expect((await response.json()).provider.id).toBe('original')
    expect(await dbProviderStore.list()).toHaveLength(1)
    expect((await dbProviderStore.getDefault())?.id).toBe('original')
  })

  it('collapses OAuth duplicates atomically without detaching scheduled jobs', async () => {
    for (const id of ['first', 'duplicate']) {
      getDb()
        .insert(providers)
        .values({
          ...config,
          id,
          type: 'chatgpt-pro',
          kind: 'llm',
          createdAt: 1,
          updatedAt: 1,
          isDefault: id === 'duplicate',
        })
        .run()
    }
    getDb()
      .insert(scheduledJobs)
      .values({
        id: 'job',
        name: 'Daily',
        query: 'Read news',
        scheduleType: 'daily',
        providerId: 'duplicate',
        createdAt: 1,
        updatedAt: 1,
      })
      .run()
    const results = await Promise.all([
      crud.request('/first', {
        method: 'PUT',
        ...json({ ...config, type: 'chatgpt-pro' }),
      }),
      crud.request('/new', {
        method: 'PUT',
        ...json({ ...config, type: 'chatgpt-pro' }),
      }),
    ])
    expect(results.map((r) => r.status)).toEqual([200, 200])
    expect(await dbProviderStore.list()).toHaveLength(1)
    expect((await dbProviderStore.getDefault())?.id).toBe('first')
    expect(getDb().select().from(scheduledJobs).get()?.providerId).toBe('first')
  })

  it('accepts explicit replacement credentials at a changed destination', async () => {
    await crud.request('/provider', { method: 'PUT', ...json(config) })
    const response = await crud.request('/provider', {
      method: 'PUT',
      ...json({
        ...config,
        baseUrl: 'https://proxy.example/v1',
        apiKey: 'replacement',
      }),
    })
    expect(response.status).toBe(200)
    expect(await dbProviderStore.getWithCredentials('provider')).toMatchObject({
      apiKey: 'replacement',
      baseUrl: 'https://proxy.example/v1',
    })
  })

  it('accepts equivalent trailing slashes without requiring the stored key again', async () => {
    await crud.request('/provider', { method: 'PUT', ...json(config) })
    const response = await testRoute.request('/', {
      method: 'POST',
      ...json({
        providerId: 'provider',
        provider: config.type,
        model: config.modelId,
        baseUrl: `${DEFAULT_URL}/`,
      }),
    })
    expect(response.status).toBe(200)
    expect(captured[0]?.apiKey).toBe(KEY)
  })

  it('requires AWS credentials for a new provider and preserves them only in the same region', async () => {
    const aws = { ...config, type: 'bedrock', region: 'us-east-1', apiKey: '' }
    expect(
      (await crud.request('/aws', { method: 'PUT', ...json(aws) })).status,
    ).toBe(400)
    expect(
      (
        await crud.request('/aws', {
          method: 'PUT',
          ...json({
            ...aws,
            accessKeyId: 'access',
            secretAccessKey: 'secret',
            sessionToken: 'token',
          }),
        })
      ).status,
    ).toBe(200)
    expect(
      (
        await crud.request('/aws', {
          method: 'PUT',
          ...json({ ...aws, accessKeyId: '', secretAccessKey: '' }),
        })
      ).status,
    ).toBe(200)
    expect(
      (
        await crud.request('/aws', {
          method: 'PUT',
          ...json({ ...aws, region: 'eu-west-1' }),
        })
      ).status,
    ).toBe(400)
    expect(await dbProviderStore.getWithCredentials('aws')).toMatchObject({
      accessKeyId: 'access',
      secretAccessKey: 'secret',
      sessionToken: 'token',
      region: 'us-east-1',
    })
  })

  it('does not fall back to a different provider when a saved test names a missing ID', async () => {
    await crud.request('/provider', { method: 'PUT', ...json(config) })
    const response = await testRoute.request('/', {
      method: 'POST',
      ...json({ providerId: 'missing' }),
    })
    expect(response.status).toBe(404)
    expect(captured).toHaveLength(0)
  })
})
