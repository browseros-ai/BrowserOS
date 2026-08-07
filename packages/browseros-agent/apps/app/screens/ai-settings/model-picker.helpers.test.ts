import { describe, expect, it } from 'bun:test'
import Fuse from 'fuse.js'
import { getProviderTemplate } from '../../lib/llm-providers/providerTemplates'
import type { ProviderType } from '../../lib/llm-providers/types'
import {
  getIncompleteCatalogHint,
  getModelPickerRows,
  normalizeModelId,
  servesUserLoadedModels,
  shouldOfferCustomModel,
} from './model-picker.helpers'
import { getModelsForProvider, type ModelInfo } from './models'

// Mirrors NewProviderDialog's Fuse setup so these tests exercise the real
// ranking the picker sees, not an idealized substring match.
function pickerRows(providerType: ProviderType, search: string) {
  const models = getModelsForProvider(providerType)
  const fuse = new Fuse(models, {
    keys: ['modelId'],
    threshold: 0.4,
    distance: 100,
  })
  return getModelPickerRows(search, models, (query) =>
    fuse.search(query).map((r) => r.item),
  )
}

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
    // Names the provider off the same template the dialog passes in, so this
    // asserts the sentence users actually read rather than echoing back a
    // literal the call site never produces.
    const providerName = getProviderTemplate('lmstudio')?.name

    expect(getIncompleteCatalogHint('lmstudio', 3, providerName)).toBe(
      `${providerName} lists only common models — paste the exact ID of any model you have loaded.`,
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

describe('getModelPickerRows', () => {
  it('offers the free-form row for an unlisted ID', () => {
    const rows = pickerRows('lmstudio', 'mistralai/magistral-small-2509')

    expect(rows.customModelId).toBe('mistralai/magistral-small-2509')
  })

  it('ranks a pasted catalog ID above its longer near-matches', () => {
    // A trailing space used to reach Fuse verbatim, where the padded pattern
    // scored better against `gpt-5.5` than against the exact `gpt-5` — and
    // with no custom row to outrank it, Enter saved the wrong model.
    for (const search of ['gpt-5 ', ' gpt-5', 'gpt-5\n', 'gpt-5\t']) {
      const rows = pickerRows('openai', search)

      expect(rows.customModelId).toBeNull()
      expect(rows.models[0]?.modelId).toBe('gpt-5')
    }
  })

  it('still resolves a catalog ID padded on both sides', () => {
    // ` o3 ` matched nothing at all, emptying a list whose trimmed form is a
    // direct hit and leaving Enter with no row to commit.
    const rows = pickerRows('openai', '  o3  ')

    expect(rows.customModelId).toBeNull()
    expect(rows.models[0]?.modelId).toBe('o3')
  })

  it('shows the whole catalog before the user types', () => {
    const rows = pickerRows('lmstudio', '')

    expect(rows.customModelId).toBeNull()
    expect(rows.models).toEqual(getModelsForProvider('lmstudio'))
  })

  it('always leaves a row for Enter to commit, which is why there is no empty state', () => {
    const providers: ProviderType[] = [
      'lmstudio',
      'openai',
      'anthropic',
      'google',
      'openrouter',
    ]

    for (const providerType of providers) {
      for (const model of getModelsForProvider(providerType)) {
        for (const search of [model.modelId, ` ${model.modelId} `]) {
          const rows = pickerRows(providerType, search)

          expect(
            rows.models.length + (rows.customModelId ? 1 : 0),
          ).toBeGreaterThan(0)
        }
      }

      const rows = pickerRows(providerType, 'definitely-not-a-real-model')
      expect(rows.customModelId).toBe('definitely-not-a-real-model')
    }
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
