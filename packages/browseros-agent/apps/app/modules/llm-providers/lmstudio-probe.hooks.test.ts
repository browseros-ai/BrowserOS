import { describe, expect, it } from 'bun:test'
import {
  isLMStudioProbeEnabled,
  parseLMStudioModels,
  toLMStudioOrigin,
} from './lmstudio-probe.hooks'

describe('toLMStudioOrigin', () => {
  it('derives the origin from a base URL with a path', () => {
    expect(toLMStudioOrigin('http://localhost:1234/v1')).toBe(
      'http://localhost:1234',
    )
  })

  it('returns undefined for an empty base URL', () => {
    expect(toLMStudioOrigin('')).toBeUndefined()
    expect(toLMStudioOrigin(undefined)).toBeUndefined()
  })

  it('returns undefined for an unparsable base URL', () => {
    expect(toLMStudioOrigin('not a url')).toBeUndefined()
  })
})

describe('isLMStudioProbeEnabled', () => {
  it('disables for non-lmstudio provider types', () => {
    expect(
      isLMStudioProbeEnabled({
        providerType: 'ollama',
        baseUrl: 'http://localhost:11434/v1',
      }),
    ).toBe(false)
  })

  it('disables when baseUrl is missing', () => {
    expect(
      isLMStudioProbeEnabled({ providerType: 'lmstudio', baseUrl: undefined }),
    ).toBe(false)
  })

  it('disables when explicit enabled flag is false', () => {
    expect(
      isLMStudioProbeEnabled({
        providerType: 'lmstudio',
        baseUrl: 'http://localhost:1234/v1',
        enabled: false,
      }),
    ).toBe(false)
  })

  it('enables for lmstudio with a parsable baseUrl', () => {
    expect(
      isLMStudioProbeEnabled({
        providerType: 'lmstudio',
        baseUrl: 'http://localhost:1234/v1',
      }),
    ).toBe(true)
  })
})

describe('parseLMStudioModels', () => {
  it('excludes embedding models', () => {
    const result = parseLMStudioModels([
      { id: 'chat-model', type: 'llm', max_context_length: 8192 },
      { id: 'embed-model', type: 'embeddings', max_context_length: 2048 },
    ])
    expect(result.map((m) => m.modelId)).toEqual(['chat-model'])
  })

  it('prefers loaded_context_length over max_context_length', () => {
    const result = parseLMStudioModels([
      {
        id: 'chat-model',
        type: 'llm',
        state: 'loaded',
        max_context_length: 131072,
        loaded_context_length: 32768,
      },
    ])
    expect(result[0]?.contextLength).toBe(32768)
  })

  it('falls back to 0 when no context length is reported', () => {
    const result = parseLMStudioModels([{ id: 'chat-model', type: 'llm' }])
    expect(result[0]?.contextLength).toBe(0)
  })

  it('sorts loaded models before not-loaded, then alphabetically', () => {
    const result = parseLMStudioModels([
      { id: 'zebra', type: 'llm', state: 'not-loaded' },
      { id: 'apple', type: 'llm', state: 'loaded' },
      { id: 'mango', type: 'vlm', state: 'not-loaded' },
    ])
    expect(result.map((m) => m.modelId)).toEqual(['apple', 'mango', 'zebra'])
  })
})
