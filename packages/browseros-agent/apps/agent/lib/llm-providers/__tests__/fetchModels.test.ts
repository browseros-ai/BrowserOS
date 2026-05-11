import { afterAll, afterEach, describe, expect, it } from 'bun:test'
import { fetchModelsFromApi, mergeModelLists } from '../fetchModels'
import type { ModelEntry } from '../types'

const originalFetch = globalThis.fetch

afterAll(() => {
  globalThis.fetch = originalFetch
})

function mockFetchJson(body: any, status = 200): typeof globalThis.fetch {
  return (async () => new Response(JSON.stringify(body), { status })) as any
}

function mockFetchError(message: string): typeof globalThis.fetch {
  return (async () => {
    throw new Error(message)
  }) as any
}

describe('fetchModelsFromApi', () => {
  afterEach(() => {
    globalThis.fetch = originalFetch
  })

  it('parses OpenAI-format response (json.data)', async () => {
    globalThis.fetch = mockFetchJson({
      data: [{ id: 'gpt-5.4' }, { id: 'gpt-5.4-mini' }],
    })

    const result = await fetchModelsFromApi(
      'https://api.openai.com/v1',
      'sk-test',
    )

    expect(result.success).toBe(true)
    expect(result.models).toHaveLength(2)
    expect(result.models[0].id).toBe('gpt-5.4')
    expect(result.models[0].source).toBe('fetched')
    expect(result.models[0].contextLength).toBe(0)
    expect(result.models[0].fetchedAt).toBeGreaterThan(0)
    expect(result.error).toBeUndefined()
  })

  it('parses alternative format (json.models)', async () => {
    globalThis.fetch = mockFetchJson({
      models: [{ model: 'llama-4' }, { name: 'mistral-large' }],
    })

    const result = await fetchModelsFromApi('https://example.com/v1')

    expect(result.success).toBe(true)
    expect(result.models).toHaveLength(2)
    expect(result.models[0].id).toBe('llama-4')
    expect(result.models[1].id).toBe('mistral-large')
  })

  it('returns error on HTTP failure', async () => {
    globalThis.fetch = mockFetchJson({ error: 'unauthorized' }, 401)

    const result = await fetchModelsFromApi(
      'https://api.openai.com/v1',
      'bad-key',
    )

    expect(result.success).toBe(false)
    expect(result.models).toHaveLength(0)
    expect(result.error).toBe('HTTP 401')
  })

  it('returns error on network failure', async () => {
    globalThis.fetch = mockFetchError('ECONNREFUSED')

    const result = await fetchModelsFromApi(
      'https://unreachable.example.com/v1',
    )

    expect(result.success).toBe(false)
    expect(result.models).toHaveLength(0)
    expect(result.error).toBe('ECONNREFUSED')
  })

  it('returns empty models array on empty data', async () => {
    globalThis.fetch = mockFetchJson({ data: [] })

    const result = await fetchModelsFromApi('https://api.example.com/v1')

    expect(result.success).toBe(true)
    expect(result.models).toHaveLength(0)
  })

  it('strips trailing slash from baseUrl', async () => {
    let requestedUrl = ''
    globalThis.fetch = (async (input: any) => {
      requestedUrl = typeof input === 'string' ? input : input.url
      return new Response(JSON.stringify({ data: [{ id: 'test' }] }))
    }) as any

    await fetchModelsFromApi('https://api.example.com/v1/')

    expect(requestedUrl).toBe('https://api.example.com/v1/models')
  })
})

describe('mergeModelLists', () => {
  const makeEntry = (
    id: string,
    source: ModelEntry['source'] = 'static',
  ): ModelEntry => ({
    id,
    contextLength: 100,
    source,
    fetchedAt: source === 'fetched' ? Date.now() : undefined,
  })

  it('deduplicates overlapping entries', () => {
    const existing = [
      makeEntry('gpt-5.4', 'static'),
      makeEntry('claude-sonnet-4', 'manual'),
    ]
    const incoming = [
      makeEntry('gpt-5.4', 'fetched'),
      makeEntry('llama-4', 'fetched'),
    ]

    const result = mergeModelLists(existing, incoming)

    expect(result).toHaveLength(3)
    const ids = result.map((m) => m.id)
    expect(ids).toContain('gpt-5.4')
    expect(ids).toContain('claude-sonnet-4')
    expect(ids).toContain('llama-4')
    // Existing entry preserved (not overwritten)
    expect(result[0].source).toBe('static')
  })

  it('returns all incoming when existing is empty', () => {
    const incoming = [
      makeEntry('model-a', 'fetched'),
      makeEntry('model-b', 'fetched'),
    ]

    const result = mergeModelLists([], incoming)

    expect(result).toHaveLength(2)
    expect(result[0].id).toBe('model-a')
    expect(result[1].id).toBe('model-b')
  })

  it('preserves all existing entries when incoming is empty', () => {
    const existing = [makeEntry('a'), makeEntry('b'), makeEntry('c')]

    const result = mergeModelLists(existing, [])

    expect(result).toHaveLength(3)
    expect(result.map((m) => m.id)).toEqual(['a', 'b', 'c'])
  })
})
