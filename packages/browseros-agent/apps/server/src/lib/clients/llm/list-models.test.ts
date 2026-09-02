import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { listProviderModels } from './list-models'

let originalFetch: typeof globalThis.fetch
let lastUrl: string | null = null
let lastHeaders: HeadersInit | undefined

beforeEach(() => {
  originalFetch = globalThis.fetch
  lastUrl = null
  lastHeaders = undefined
})

afterEach(() => {
  globalThis.fetch = originalFetch
})

function mockFetchJson(
  body: unknown,
  init: { status?: number; statusText?: string } = {},
) {
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    lastUrl = typeof input === 'string' ? input : input.toString()
    return {
      ok: (init.status ?? 200) < 400,
      status: init.status ?? 200,
      statusText: init.statusText ?? '',
      json: async () => body,
    } as unknown as Response
  }) as unknown as typeof globalThis.fetch
}

describe('listProviderModels — happy path', () => {
  test('maps id and context_length', async () => {
    mockFetchJson({
      data: [{ id: 'llama3.2', context_length: 131072 }, { id: 'qwen2.5' }],
    })
    const models = await listProviderModels({
      provider: 'openai-compatible',
      baseUrl: 'http://localhost:11434',
    })
    expect(models).toEqual([
      { modelId: 'llama3.2', contextLength: 131072 },
      { modelId: 'qwen2.5', contextLength: undefined },
    ])
    expect(lastUrl).toBe('http://localhost:11434/v1/models')
  })
})

describe('listProviderModels — response shape', () => {
  test('missing data yields []', async () => {
    mockFetchJson({})
    const models = await listProviderModels({
      provider: 'lmstudio',
      baseUrl: 'http://localhost:1234',
    })
    expect(models).toEqual([])
  })

  test('filters non-string ids', async () => {
    mockFetchJson({ data: [{ id: 42 }, { id: 'good' }, {}] })
    const models = await listProviderModels({
      provider: 'ollama',
      baseUrl: 'http://localhost:11434',
    })
    expect(models).toEqual([{ modelId: 'good', contextLength: undefined }])
  })
})

describe('listProviderModels — failures', () => {
  test('throws on 401', async () => {
    mockFetchJson(
      { error: 'bad key' },
      { status: 401, statusText: 'Unauthorized' },
    )
    expect(
      listProviderModels({
        provider: 'openai-compatible',
        baseUrl: 'http://x',
      }),
    ).rejects.toThrow('[openai-compatible] 401 Unauthorized')
  })

  test('throws on 500', async () => {
    mockFetchJson({}, { status: 500, statusText: 'Internal Server Error' })
    expect(
      listProviderModels({
        provider: 'openai-compatible',
        baseUrl: 'http://x',
      }),
    ).rejects.toThrow('500')
  })

  test('throws when baseUrl is missing', async () => {
    mockFetchJson({ data: [] })
    expect(
      listProviderModels({ provider: 'openai-compatible' }),
    ).rejects.toThrow('baseUrl is required')
  })
})

describe('listProviderModels — URL normalization', () => {
  test.each(['http://x/v1', 'http://x/v1/', 'http://x/', 'http://x'])(
    '%s → http://x/v1/models',
    async (baseUrl) => {
      mockFetchJson({ data: [] })
      await listProviderModels({ provider: 'p', baseUrl })
      expect(lastUrl).toBe('http://x/v1/models')
    },
  )
})

describe('listProviderModels — auth header', () => {
  test('sends Bearer token when apiKey present', async () => {
    mockFetchJson({ data: [] })
    globalThis.fetch = (async (
      input: RequestInfo | URL,
      init?: RequestInit,
    ) => {
      lastUrl = input.toString()
      lastHeaders = init?.headers
      return {
        ok: true,
        status: 200,
        json: async () => ({ data: [] }),
      } as Response
    }) as unknown as typeof globalThis.fetch

    await listProviderModels({
      provider: 'p',
      baseUrl: 'http://x',
      apiKey: 'sk-test',
    })
    expect(lastHeaders).toEqual({ Authorization: 'Bearer sk-test' })
  })
})
