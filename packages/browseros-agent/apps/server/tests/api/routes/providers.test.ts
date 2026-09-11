import { describe, expect, it } from 'bun:test'
import { createProvidersRoutes } from '../../../src/api/routes/providers'
import type { ProviderRow } from '../../../src/lib/db/schema'
import type {
  ProviderStore,
  ProviderUpsert,
} from '../../../src/lib/providers/provider-store'

const PROVIDER_ID = 'provider-1'

function row(overrides: Partial<ProviderRow> = {}): ProviderRow {
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

function memoryStore(initial: ProviderRow[] = []) {
  const rows = new Map(initial.map((r) => [r.id, r]))
  const store: ProviderStore = {
    list: async () => [...rows.values()],
    get: async (id) => rows.get(id) ?? null,
    upsert: async (input: ProviderUpsert) => {
      const existing = rows.get(input.id)
      const saved = {
        ...row(),
        ...input,
        createdAt: existing?.createdAt ?? input.createdAt ?? 100,
        updatedAt: 200,
      } as ProviderRow
      rows.set(saved.id, saved)
      return saved
    },
    insertIfAbsent: async (input: ProviderUpsert) => {
      if (rows.has(input.id)) return null
      return store.upsert(input)
    },
    remove: async (id) => rows.delete(id),
    listLlm: async () => [...rows.values()].filter((row) => row.kind === 'llm'),
    getDefault: async () =>
      [...rows.values()].find((row) => row.isDefault) ?? null,
    setDefault: async (id) => {
      if (!rows.has(id)) return false
      for (const row of rows.values()) row.isDefault = row.id === id
      return true
    },
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
  it('round-trips custom headers through provider writes and reads', async () => {
    const routes = createProvidersRoutes(memoryStore())
    const headers = { 'x-opencode-session': '{{conversationId}}' }
    const response = await routes.request(`/${PROVIDER_ID}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...body, headers }),
    })
    expect(response.status).toBe(200)
    expect(
      await (await routes.request(`/${PROVIDER_ID}`)).json(),
    ).toMatchObject({ provider: { headers } })
  })

  it('rejects malformed custom headers before saving', async () => {
    const { store, rows } = memoryStore()
    const routes = createProvidersRoutes({ store })
    const response = await routes.request(`/${PROVIDER_ID}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...body, headers: { 'x-test': 'a\r\nb' } }),
    })
    expect(response.status).toBe(400)
    expect(rows.size).toBe(0)
  })

  it('lists providers', async () => {
    const routes = createProvidersRoutes(memoryStore([row()]))
    const response = await routes.request('/')
    expect(response.status).toBe(200)
    expect(await response.json()).toMatchObject({
      providers: [{ id: PROVIDER_ID, name: 'My OpenAI' }],
    })
  })

  it('gets one provider', async () => {
    const routes = createProvidersRoutes(memoryStore([row()]))
    const response = await routes.request(`/${PROVIDER_ID}`)
    expect(response.status).toBe(200)
    expect(await response.json()).toMatchObject({
      provider: { id: PROVIDER_ID },
    })
  })

  it('returns 404 for an unknown provider', async () => {
    const routes = createProvidersRoutes(memoryStore())
    expect((await routes.request(`/${PROVIDER_ID}`)).status).toBe(404)
  })

  it('creates a provider under the id from the path', async () => {
    const { store, rows } = memoryStore()
    const routes = createProvidersRoutes({ store })

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
    const routes = createProvidersRoutes({ store })
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
    const routes = createProvidersRoutes({ store })

    await routes.request(`/${PROVIDER_ID}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...body, createdAt: 999 }),
    })

    expect(rows.get(PROVIDER_ID)?.createdAt).toBe(42)
  })

  it('rejects a body missing required fields', async () => {
    const routes = createProvidersRoutes(memoryStore())
    const response = await routes.request(`/${PROVIDER_ID}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'no type or model' }),
    })
    expect(response.status).toBe(400)
  })

  it('deletes a provider', async () => {
    const { store, rows } = memoryStore([row()])
    const routes = createProvidersRoutes({ store })

    const response = await routes.request(`/${PROVIDER_ID}`, {
      method: 'DELETE',
    })
    expect(response.status).toBe(200)
    expect(rows.size).toBe(0)
  })

  it('returns 404 deleting an unknown provider', async () => {
    const routes = createProvidersRoutes(memoryStore())
    expect(
      (await routes.request(`/${PROVIDER_ID}`, { method: 'DELETE' })).status,
    ).toBe(404)
  })

  // Credentials are the reason this table exists rather than staying remote.
  it('round-trips credentials, which the cloud never carried', async () => {
    const { store, rows } = memoryStore()
    const routes = createProvidersRoutes({ store })

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

  describe('import', () => {
    async function importProviders(
      routes: ReturnType<typeof createProvidersRoutes>,
      providers: unknown[],
    ) {
      return routes.request('/import', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ providers }),
      })
    }

    it('inserts a provider that is not there yet', async () => {
      const { store, rows } = memoryStore()
      const routes = createProvidersRoutes({ store })
      const response = await importProviders(routes, [
        { ...body, id: PROVIDER_ID },
      ])

      expect(response.status).toBe(200)
      expect(await response.json()).toEqual({
        imported: [PROVIDER_ID],
        skipped: [],
      })
      expect(rows.get(PROVIDER_ID)).toMatchObject({ apiKey: 'sk-test' })
    })

    // The whole reason import is insert-if-absent: the app writes here
    // directly, so a second run must not restore the pre-edit copy that is
    // still sitting in extension storage.
    it('leaves an existing provider untouched and reports it skipped', async () => {
      const { store, rows } = memoryStore([row({ name: 'Edited since' })])
      const routes = createProvidersRoutes({ store })
      const response = await importProviders(routes, [
        { ...body, id: PROVIDER_ID, name: 'Stale copy' },
      ])

      expect(await response.json()).toEqual({
        imported: [],
        skipped: [PROVIDER_ID],
      })
      expect(rows.get(PROVIDER_ID)?.name).toBe('Edited since')
    })

    it('partitions a mixed batch', async () => {
      const { store } = memoryStore([row()])
      const routes = createProvidersRoutes({ store })
      const response = await importProviders(routes, [
        { ...body, id: PROVIDER_ID },
        { ...body, id: 'provider-2' },
      ])

      expect(await response.json()).toEqual({
        imported: ['provider-2'],
        skipped: [PROVIDER_ID],
      })
    })

    it('rejects a provider with no id', async () => {
      const routes = createProvidersRoutes(memoryStore())
      expect((await importProviders(routes, [body])).status).toBe(400)
    })
  })

  describe('default', () => {
    it('reports no default when none is set', async () => {
      const routes = createProvidersRoutes(memoryStore([row()]))
      const response = await routes.request('/default')

      expect(response.status).toBe(200)
      expect(await response.json()).toEqual({ provider: null })
    })

    it('points the default at a provider', async () => {
      const { store, rows } = memoryStore([row()])
      const routes = createProvidersRoutes({ store })

      const response = await routes.request('/default', {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ providerId: PROVIDER_ID }),
      })

      expect(response.status).toBe(200)
      expect(rows.get(PROVIDER_ID)?.isDefault).toBe(true)
    })

    it('refuses an unknown provider rather than storing a stale pointer', async () => {
      const routes = createProvidersRoutes(memoryStore())
      const response = await routes.request('/default', {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ providerId: 'nope' }),
      })

      expect(response.status).toBe(404)
    })

    // /default is declared before /:providerId, so the literal path is not
    // swallowed as an id.
    it('does not read the default path as a provider id', async () => {
      const { store } = memoryStore([row({ id: 'default' })])
      const routes = createProvidersRoutes({ store })

      expect(await (await routes.request('/default')).json()).toEqual({
        provider: null,
      })
    })
  })
})
