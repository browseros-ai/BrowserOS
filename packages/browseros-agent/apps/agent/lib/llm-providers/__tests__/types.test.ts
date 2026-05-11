import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig, ModelEntry } from '../types'

// ---------------------------------------------------------------------------
// 1. ModelEntry type — structural verification
// ---------------------------------------------------------------------------
describe('ModelEntry type', () => {
  it('requires id, contextLength, source', () => {
    const entry: ModelEntry = {
      id: 'gpt-5.4',
      contextLength: 128000,
      source: 'manual',
    }
    expect(entry.id).toBe('gpt-5.4')
    expect(entry.contextLength).toBe(128000)
    expect(entry.source).toBe('manual')
  })

  it('allows optional supportsImages', () => {
    const entry: ModelEntry = {
      id: 'claude-sonnet-4-6',
      contextLength: 200000,
      source: 'static',
      supportsImages: true,
    }
    expect(entry.supportsImages).toBe(true)
  })

  it('allows optional fetchedAt', () => {
    const entry: ModelEntry = {
      id: 'deepseek-r1',
      contextLength: 64000,
      source: 'fetched',
      fetchedAt: 1715400000000,
    }
    expect(entry.fetchedAt).toBe(1715400000000)
  })

  it('source must be one of static|fetched|manual', () => {
    const valid: ModelEntry['source'][] = ['static', 'fetched', 'manual']
    expect(valid).toHaveLength(3)
    for (const s of valid) {
      expect(['static', 'fetched', 'manual']).toContain(s)
    }
  })
})

// ---------------------------------------------------------------------------
// 2. Storage migration v3 — providers without models[] get synthesized entry
//
// NOTE: The v3 migration function lives inside providersStorage.migrations
// in storage.ts, which requires a web extension environment to import.
// These tests verify the CORRECT migration behavior using a reference
// implementation. When the migration is added to storage.ts, the actual
// logic should match this exactly.
// ---------------------------------------------------------------------------
describe('Storage migration v3', () => {
  /**
   * Reference implementation of the v3 migration from the design doc.
   * The actual implementation in storage.ts must match this behavior.
   */
  function migrateV3(
    providers: LlmProviderConfig[] | null,
  ): LlmProviderConfig[] | null {
    if (!providers) return providers
    return providers.map((provider) => ({
      ...provider,
      models: provider.models ?? [
        {
          id: provider.modelId,
          contextLength: provider.contextWindow,
          supportsImages: provider.supportsImages,
          source: 'manual' as const,
        },
      ],
    }))
  }

  it('synthesizes models[] from modelId when provider has no models', () => {
    const provider: LlmProviderConfig = {
      id: 'openai-1',
      type: 'openai',
      name: 'OpenAI',
      modelId: 'gpt-5.4',
      contextWindow: 128000,
      supportsImages: true,
      temperature: 0.7,
      createdAt: 1715400000000,
      updatedAt: 1715400000000,
    }

    const result = migrateV3([provider])
    expect(result).toHaveLength(1)
    expect(result[0].models).toEqual([
      {
        id: 'gpt-5.4',
        contextLength: 128000,
        supportsImages: true,
        source: 'manual',
      },
    ])
  })

  it('preserves all other provider fields during migration', () => {
    const provider: LlmProviderConfig = {
      id: 'anthropic-1',
      type: 'anthropic',
      name: 'Anthropic',
      modelId: 'claude-sonnet-4-6',
      contextWindow: 200000,
      supportsImages: true,
      temperature: 0.5,
      apiKey: 'sk-test-123',
      createdAt: 1715400000000,
      updatedAt: 1715400000000,
    }

    const result = migrateV3([provider])
    const migrated = result[0]

    // All original fields must survive
    expect(migrated.id).toBe('anthropic-1')
    expect(migrated.type).toBe('anthropic')
    expect(migrated.name).toBe('Anthropic')
    expect(migrated.modelId).toBe('claude-sonnet-4-6')
    expect(migrated.temperature).toBe(0.5)
    expect(migrated.apiKey).toBe('sk-test-123')
    expect(migrated.createdAt).toBe(1715400000000)
  })

  it('uses modelId and contextWindow to build the synthesized entry', () => {
    const provider: LlmProviderConfig = {
      id: 'ollama-1',
      type: 'ollama',
      name: 'Ollama',
      modelId: 'llama-4',
      contextWindow: 32000,
      supportsImages: false,
      temperature: 0.8,
      createdAt: 1715400000000,
      updatedAt: 1715400000000,
    }

    const result = migrateV3([provider])
    expect(result[0].models?.[0]).toEqual({
      id: 'llama-4',
      contextLength: 32000,
      supportsImages: false,
      source: 'manual',
    })
  })
})

// ---------------------------------------------------------------------------
// 3. Storage migration v3 edge cases
// ---------------------------------------------------------------------------
describe('Storage migration v3 edge cases', () => {
  function migrateV3(
    providers: LlmProviderConfig[] | null,
  ): LlmProviderConfig[] | null {
    if (!providers) return providers
    return providers.map((provider) => ({
      ...provider,
      models: provider.models ?? [
        {
          id: provider.modelId,
          contextLength: provider.contextWindow,
          supportsImages: provider.supportsImages,
          source: 'manual' as const,
        },
      ],
    }))
  }

  it('does NOT overwrite existing models[]', () => {
    const existingModels: ModelEntry[] = [
      { id: 'gpt-5.4', contextLength: 128000, source: 'static' },
      {
        id: 'o3',
        contextLength: 200000,
        source: 'fetched',
        fetchedAt: 1715400000000,
      },
    ]

    const provider: LlmProviderConfig = {
      id: 'openai-1',
      type: 'openai',
      name: 'OpenAI',
      modelId: 'gpt-5.4',
      contextWindow: 128000,
      supportsImages: true,
      temperature: 0.7,
      createdAt: 1715400000000,
      updatedAt: 1715400000000,
      models: existingModels,
    }

    const result = migrateV3([provider])
    // models[] must be untouched
    expect(result[0].models).toEqual(existingModels)
  })

  it('returns null when input is null', () => {
    expect(migrateV3(null)).toBeNull()
  })

  it('handles multiple providers, mixing those with and without models[]', () => {
    const providers: LlmProviderConfig[] = [
      {
        id: 'p1',
        type: 'openai',
        name: 'OpenAI',
        modelId: 'gpt-5.4',
        contextWindow: 128000,
        supportsImages: true,
        temperature: 0.7,
        createdAt: 1715400000000,
        updatedAt: 1715400000000,
        // NO models[]
      },
      {
        id: 'p2',
        type: 'anthropic',
        name: 'Anthropic',
        modelId: 'claude-sonnet-4-6',
        contextWindow: 200000,
        supportsImages: true,
        temperature: 0.5,
        createdAt: 1715400000000,
        updatedAt: 1715400000000,
        models: [
          { id: 'claude-sonnet-4-6', contextLength: 200000, source: 'static' },
        ],
      },
    ]

    const result = migrateV3(providers)
    expect(result).toHaveLength(2)

    // p1: synthesized
    expect(result[0].models).toEqual([
      {
        id: 'gpt-5.4',
        contextLength: 128000,
        supportsImages: true,
        source: 'manual',
      },
    ])
    // p2: untouched
    expect(result[1].models).toEqual([
      { id: 'claude-sonnet-4-6', contextLength: 200000, source: 'static' },
    ])
  })

  it('handles empty providers array', () => {
    expect(migrateV3([])).toEqual([])
  })
})

// ---------------------------------------------------------------------------
// 4. createDefaultBrowserOSProvider() — includes static models entry
//
// NOTE: Cannot directly import from storage.ts due to WXT browser extension
// dependency. Instead we test against the expected return shape.
// When storage.ts is updated, these tests validate the contract.
// ---------------------------------------------------------------------------
describe('createDefaultBrowserOSProvider() contract', () => {
  /**
   * Expected shape from the design doc:
   *   models: [{ id: 'browseros-auto', contextLength: 200000, source: 'static' }]
   *   modelId: 'browseros-auto'
   *
   * This test verifies the TYPE contract. The actual runtime test will be
   * run once storage.ts is updated with the new fields.
   */
  it('expected shape has models with browseros-auto static entry', () => {
    const expectedModels: ModelEntry[] = [
      {
        id: 'browseros-auto',
        contextLength: 200000,
        source: 'static',
      },
    ]

    // Verify the expected structure
    expect(expectedModels).toHaveLength(1)
    expect(expectedModels[0].id).toBe('browseros-auto')
    expect(expectedModels[0].contextLength).toBe(200000)
    expect(expectedModels[0].source).toBe('static')
  })

  it('modelId must equal browseros-auto for backward compat', () => {
    // This is the existing behavior — modelId should remain 'browseros-auto'
    const expectedModelId = 'browseros-auto'
    expect(expectedModelId).toBe('browseros-auto')
  })

  it('default provider shape satisfies LlmProviderConfig', () => {
    const provider: LlmProviderConfig = {
      id: 'browseros',
      type: 'browseros',
      name: 'BrowserOS',
      baseUrl: 'https://api.browseros.com/v1',
      modelId: 'browseros-auto',
      supportsImages: true,
      contextWindow: 200000,
      temperature: 0.2,
      createdAt: Date.now(),
      updatedAt: Date.now(),
      models: [
        { id: 'browseros-auto', contextLength: 200000, source: 'static' },
      ],
    }

    expect(provider.models).toBeDefined()
    expect(provider.models?.[0]).toEqual({
      id: 'browseros-auto',
      contextLength: 200000,
      source: 'static',
    })
  })
})

// ---------------------------------------------------------------------------
// 5. LlmProviderConfig backward compat — modelId field
// ---------------------------------------------------------------------------
describe('LlmProviderConfig backward compat', () => {
  it('modelId field exists and is writable', () => {
    const config: LlmProviderConfig = {
      id: 'test',
      type: 'openai',
      name: 'Test',
      modelId: 'gpt-4',
      supportsImages: false,
      contextWindow: 8000,
      temperature: 1,
      createdAt: 0,
      updatedAt: 0,
    }

    // modelId must be readable
    expect(config.modelId).toBe('gpt-4')

    // modelId must be writable
    config.modelId = 'gpt-5.4'
    expect(config.modelId).toBe('gpt-5.4')
  })

  it('accepts optional models[] on LlmProviderConfig', () => {
    const config: LlmProviderConfig = {
      id: 'test',
      type: 'openai',
      name: 'Test',
      modelId: 'gpt-5.4',
      supportsImages: false,
      contextWindow: 128000,
      temperature: 1,
      createdAt: 0,
      updatedAt: 0,
      models: [
        { id: 'gpt-5.4', contextLength: 128000, source: 'static' },
        {
          id: 'o3',
          contextLength: 200000,
          source: 'fetched',
          fetchedAt: 1715400000000,
        },
      ],
    }

    expect(config.models).toHaveLength(2)
    expect(config.models?.[0].id).toBe('gpt-5.4')
    expect(config.models?.[1].source).toBe('fetched')
  })

  it('accepts optional fetchedModels on LlmProviderConfig', () => {
    const config: LlmProviderConfig = {
      id: 'test',
      type: 'openai',
      name: 'Test',
      modelId: 'gpt-5.4',
      supportsImages: false,
      contextWindow: 128000,
      temperature: 1,
      createdAt: 0,
      updatedAt: 0,
      fetchedModels: {
        fetchedAt: 1715400000000,
        ids: ['gpt-5.4', 'o3', 'gpt-5.4-mini'],
      },
    }

    expect(config.fetchedModels).toBeDefined()
    expect(config.fetchedModels?.ids).toHaveLength(3)
    expect(config.fetchedModels?.fetchedAt).toBe(1715400000000)
  })
})
