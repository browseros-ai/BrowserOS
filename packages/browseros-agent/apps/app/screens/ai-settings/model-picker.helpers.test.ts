import { describe, expect, it } from 'bun:test'
import {
  getIncompleteCatalogHint,
  normalizeModelId,
  servesUserLoadedModels,
  shouldOfferCustomModel,
} from './model-picker.helpers'
import { getModelsForProvider, type ModelInfo } from './models'

const catalog: ModelInfo[] = [
  { modelId: 'openai/gpt-oss-20b', contextLength: 131072 },
  { modelId: 'qwen/qwen3-coder-30b', contextLength: 262144 },
]

describe('normalizeModelId', () => {
  it('strips whitespace picked up when pasting from provider UIs', () => {
    expect(normalizeModelId('  qwen/qwen3-coder-30b\n')).toBe(
      'qwen/qwen3-coder-30b',
    )
  })

  it('reduces whitespace-only input to the empty string', () => {
    expect(normalizeModelId('   ')).toBe('')
  })
})

describe('shouldOfferCustomModel', () => {
  it('offers an unlisted model ID', () => {
    expect(shouldOfferCustomModel('openai/gpt-oss-120b', catalog)).toBe(true)
  })

  it('does not offer a model the catalog already lists', () => {
    expect(shouldOfferCustomModel('qwen/qwen3-coder-30b', catalog)).toBe(false)
  })

  it('does not duplicate a listed model that was pasted with whitespace', () => {
    expect(shouldOfferCustomModel(' qwen/qwen3-coder-30b ', catalog)).toBe(
      false,
    )
  })

  it('offers nothing until the user types something usable', () => {
    expect(shouldOfferCustomModel('', catalog)).toBe(false)
    expect(shouldOfferCustomModel('  ', catalog)).toBe(false)
  })

  it('offers free-form entry when the provider has no catalog at all', () => {
    expect(shouldOfferCustomModel('llama3.2', [])).toBe(true)
  })
})

describe('servesUserLoadedModels', () => {
  it('covers the endpoints that serve locally loaded or proxied models', () => {
    expect(servesUserLoadedModels('lmstudio')).toBe(true)
    expect(servesUserLoadedModels('ollama')).toBe(true)
    expect(servesUserLoadedModels('openai-compatible')).toBe(true)
  })

  it('excludes providers whose catalog is authoritative', () => {
    expect(servesUserLoadedModels('anthropic')).toBe(false)
    expect(servesUserLoadedModels('openai')).toBe(false)
  })
})

describe('getIncompleteCatalogHint', () => {
  it('warns that a local runtime catalog is only a sample', () => {
    expect(getIncompleteCatalogHint('lmstudio', 3, 'LM Studio')).toContain(
      'LM Studio lists only common models',
    )
  })

  it('stays silent for providers with an authoritative catalog', () => {
    expect(getIncompleteCatalogHint('anthropic', 12, 'Anthropic')).toBeNull()
  })

  it('stays silent when the field already renders as free-form input', () => {
    expect(getIncompleteCatalogHint('ollama', 0, 'Ollama')).toBeNull()
  })

  it('falls back to a generic subject when the provider has no display name', () => {
    expect(getIncompleteCatalogHint('lmstudio', 3)).toContain('This provider')
  })
})

describe('LM Studio model catalog', () => {
  // Regression guard for the bug report: LM Studio ships a handful of
  // models.dev entries, so the picker must never present that list as the
  // complete set of what the endpoint can serve.
  it('is short enough that free-form entry has to stay discoverable', () => {
    const models = getModelsForProvider('lmstudio')

    expect(models.length).toBeGreaterThan(0)
    expect(
      getIncompleteCatalogHint('lmstudio', models.length, 'LM Studio'),
    ).not.toBeNull()
    expect(shouldOfferCustomModel('qwen/qwen3-vl-8b', models)).toBe(true)
  })
})
