import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { listModels } from './listModels'

let originalFetch: typeof globalThis.fetch

beforeEach(() => {
  originalFetch = globalThis.fetch
})

afterEach(() => {
  globalThis.fetch = originalFetch
})

function mockFetchJson(body: unknown, ok = true) {
  globalThis.fetch = (async () =>
    ({
      ok,
      json: async () => body,
    }) as unknown as Response) as unknown as typeof globalThis.fetch
}

describe('listModels — success', () => {
  it('maps modelId and contextLength', async () => {
    mockFetchJson({
      models: [{ modelId: 'llama3.2', contextLength: 131072 }],
    })
    const models = await listModels(
      { type: 'ollama', baseUrl: 'http://localhost:11434' },
      'http://127.0.0.1:9200',
    )
    expect(models).toEqual([{ modelId: 'llama3.2', contextLength: 131072 }])
  })

  it('defaults contextLength to 128000 when missing', async () => {
    mockFetchJson({ models: [{ modelId: 'qwen2.5' }] })
    const models = await listModels(
      { type: 'lmstudio', baseUrl: 'http://localhost:1234' },
      'http://127.0.0.1:9200',
    )
    expect(models).toEqual([{ modelId: 'qwen2.5', contextLength: 128000 }])
  })
})

describe('listModels — soft failures return []', () => {
  it('network failure → []', async () => {
    globalThis.fetch = (async () => {
      throw new TypeError('Failed to fetch')
    }) as unknown as typeof globalThis.fetch

    const models = await listModels(
      { type: 'openai-compatible', baseUrl: 'http://x' },
      'http://127.0.0.1:9200',
    )
    expect(models).toEqual([])
  })

  it('non-200 response → []', async () => {
    mockFetchJson({ models: [{ modelId: 'm' }] }, false)
    const models = await listModels(
      { type: 'openai-compatible', baseUrl: 'http://x' },
      'http://127.0.0.1:9200',
    )
    expect(models).toEqual([])
  })
})
