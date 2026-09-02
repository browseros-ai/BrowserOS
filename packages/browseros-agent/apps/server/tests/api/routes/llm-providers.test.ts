import { describe, expect, it } from 'bun:test'
import { createLlmProviderRoutes } from '../../../src/api/routes/llm-providers'
import type { LlmProviderRow } from '../../../src/lib/db/schema'
import type {
  LlmProviderStore,
  LlmProviderUpsert,
} from '../../../src/lib/llm-providers/provider-store'

const PROVIDER_ID = 'provider-1'

function row(overrides: Partial<LlmProviderRow> = {}): LlmProviderRow {
  return {
    id: PROVIDER_ID,
    profileId: null,
    type: 'openai',
    name: 'My OpenAI',
    baseUrl: 'https://api.openai.com/v1',
    modelId: 'gpt-5.5',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    apiKey: 'sk-test',
    accessKeyId: null,
    secretAccessKey: null,
    sessionToken: null,
    resourceName: null,
    region: null,
    reasoningEffort: null,
    reasoningSummary: null,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  }
}

function memoryStore(initial: LlmProviderRow[] = []) {
  const rows = new Map(initial.map((r) => [r.id, r]))
  const store: LlmProviderStore = {
    list: async () => [...rows.values()],
    get: async (id) => rows.get(id) ?? null,
    upsert: async (input: LlmProviderUpsert) => {
      const existing = rows.get(input.id)
      const saved = {
        ...row(),
        ...input,
        createdAt: existing?.createdAt ?? input.createdAt ?? 100,
        updatedAt: 200,
      } as LlmProviderRow
      rows.set(saved.id, saved)
      return saved
    },
    remove: async (id) => rows.delete(id),
  }
  return { store, rows }
}

const body = {
  type: 'openai',
  name: 'My OpenAI',
  modelId: 'gpt-5.5',
  contextWindow: 200000,
  apiKey: 'sk-test',
}

describe('llm provider routes', () => {
  it('lists providers', async () => {
    const routes = createLlmProviderRoutes(memoryStore([row()]))
    const response = await routes.request('/')
    expect(response.status).toBe(200)
    expect(await response.json()).toMatchObject({
      providers: [{ id: PROVIDER_ID, name: 'My OpenAI' }],
    })
  })

  it('gets one provider', async () => {
    const routes = createLlmProviderRoutes(memoryStore([row()]))
    const response = await routes.request(`/${PROVIDER_ID}`)
    expect(response.status).toBe(200)
    expect(await response.json()).toMatchObject({
      provider: { id: PROVIDER_ID },
    })
  })

  it('returns 404 for an unknown provider', async () => {
    const routes = createLlmProviderRoutes(memoryStore())
    expect((await routes.request(`/${PROVIDER_ID}`)).status).toBe(404)
  })

  it('creates a provider under the id from the path', async () => {
    const { store, rows } = memoryStore()
    const routes = createLlmProviderRoutes({ store })

    const response = await routes.request(`/${PROVIDER_ID}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })

    expect(response.status).toBe(200)
    expect(rows.get(PROVIDER_ID)?.name).toBe('My OpenAI')
  })

  // The migration re-runs on every profile and after a partial failure, so a
  // repeated PUT has to land on the same row rather than a second one.
  it('is idempotent: putting the same id twice keeps one row', async () => {
    const { store, rows } = memoryStore()
    const routes = createLlmProviderRoutes({ store })
    const put = () =>
      routes.request(`/${PROVIDER_ID}`, {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      })

    await put()
    await put()

    expect(rows.size).toBe(1)
  })

  it('keeps the original creation time when a provider is re-imported', async () => {
    const { store, rows } = memoryStore([row({ createdAt: 42 })])
    const routes = createLlmProviderRoutes({ store })

    await routes.request(`/${PROVIDER_ID}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...body, createdAt: 999 }),
    })

    expect(rows.get(PROVIDER_ID)?.createdAt).toBe(42)
  })

  it('rejects a body missing required fields', async () => {
    const routes = createLlmProviderRoutes(memoryStore())
    const response = await routes.request(`/${PROVIDER_ID}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'no type or model' }),
    })
    expect(response.status).toBe(400)
  })

  it('deletes a provider', async () => {
    const { store, rows } = memoryStore([row()])
    const routes = createLlmProviderRoutes({ store })

    const response = await routes.request(`/${PROVIDER_ID}`, {
      method: 'DELETE',
    })
    expect(response.status).toBe(200)
    expect(rows.size).toBe(0)
  })

  it('returns 404 deleting an unknown provider', async () => {
    const routes = createLlmProviderRoutes(memoryStore())
    expect(
      (await routes.request(`/${PROVIDER_ID}`, { method: 'DELETE' })).status,
    ).toBe(404)
  })

  // Credentials are the reason this table exists rather than staying remote.
  it('round-trips credentials, which the cloud never carried', async () => {
    const { store, rows } = memoryStore()
    const routes = createLlmProviderRoutes({ store })

    await routes.request(`/${PROVIDER_ID}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        ...body,
        type: 'bedrock',
        accessKeyId: 'AKIA',
        secretAccessKey: 'secret',
        sessionToken: 'token',
      }),
    })

    expect(rows.get(PROVIDER_ID)).toMatchObject({
      accessKeyId: 'AKIA',
      secretAccessKey: 'secret',
      sessionToken: 'token',
    })
  })
})
