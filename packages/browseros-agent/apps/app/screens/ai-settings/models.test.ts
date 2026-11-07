import { describe, expect, it } from 'bun:test'
import { getModelsDevModels } from '../../lib/llm-providers/models-dev'
import { shouldOfferCustomModel } from './model-picker.helpers'
import {
  getDefaultReasoningEffort,
  getModelContextLength,
  getModelInfo,
  getModelsForProvider,
  modelSupportsReasoning,
} from './models'

describe('MiniMax gateway choices', () => {
  it('offers verified OpenRouter Flash first, then M3', () => {
    expect(
      getModelsForProvider('openrouter')
        .filter((m) => m.modelId.startsWith('minimax/'))
        .map((m) => m.modelId),
    ).toEqual(['minimax/minimax-m3.1-flash:free', 'minimax/minimax-m3'])
    expect(
      getModelsDevModels('openrouter').find(
        (m) => m.id === 'minimax/minimax-m3.1-flash:free',
      ),
    ).toMatchObject({
      contextWindow: 1000000,
      maxOutput: 524288,
      inputCost: 0,
      outputCost: 0,
      supportsImages: true,
      supportsToolCall: true,
      reasoningControls: [
        { type: 'effort', values: ['low', 'medium', 'high', 'xhigh', 'max'] },
      ],
    })
  })

  for (const provider of ['openrouter', 'bedrock'] as const) {
    it(`preserves ${provider} legacy metadata and explicit custom entry`, () => {
      const prefix = provider === 'openrouter' ? 'minimax/' : 'minimax.'
      const legacy = getModelsDevModels(provider).filter(
        (m) => m.id.startsWith(prefix) && !m.id.includes('m3'),
      )
      expect(legacy.length).toBeGreaterThan(0)
      const choices = getModelsForProvider(provider)
      for (const model of legacy) {
        expect(choices.some((m) => m.modelId === model.id)).toBe(false)
        expect(getModelInfo(provider, model.id)).toMatchObject({
          modelId: model.id,
          contextLength: model.contextWindow,
          supportsImages: model.supportsImages,
          supportsReasoning: model.supportsReasoning,
        })
        expect(shouldOfferCustomModel(model.id, choices)).toBe(true)
      }
      expect(shouldOfferCustomModel('my-private-model', choices)).toBe(true)
    })

    it(`leaves all non-MiniMax ${provider} choices in their original order`, () => {
      expect(
        getModelsForProvider(provider)
          .filter((m) => !m.modelId.startsWith('minimax'))
          .map((m) => m.modelId),
      ).toEqual(
        getModelsDevModels(provider)
          .filter((m) => !m.id.startsWith('minimax'))
          .map((m) => m.id),
      )
    })
  }

  it('uses Flash max effort without changing other model defaults', () => {
    expect(
      getDefaultReasoningEffort(
        'openrouter',
        getModelInfo('openrouter', 'minimax/minimax-m3.1-flash:free'),
      ),
    ).toBe('max')
    expect(
      getDefaultReasoningEffort(
        'openrouter',
        getModelInfo('openrouter', 'minimax/minimax-m3'),
      ),
    ).toBe('medium')
    expect(getDefaultReasoningEffort('openai-compatible', undefined)).toBe(
      'medium',
    )
  })

  it('does not invent Bedrock Flash or M3 IDs', () => {
    expect(
      getModelsForProvider('bedrock').some((m) =>
        m.modelId.startsWith('minimax.'),
      ),
    ).toBe(false)
  })
})

describe('ChatGPT subscription models', () => {
  const models = getModelsForProvider('chatgpt-pro')

  it('offers the current generation, not a snapshot of an older one', () => {
    // The regression this replaced: the list was typed by hand, so it stayed
    // a generation behind until someone shipped an extension release. Asserted
    // as a property of the catalogue rather than against named ids, which
    // would go stale the same way.
    const generations = models
      .map((model) => /^gpt-(\d+)/.exec(model.modelId)?.[1])
      .filter((generation): generation is string => generation !== undefined)
      .map(Number)

    expect(Math.max(...generations)).toBeGreaterThanOrEqual(6)
  })

  it('keeps the codex variants the platform catalogue does not carry', () => {
    const ids = models.map((model) => model.modelId)

    expect(ids).toContain('gpt-5.1-codex-max')
    expect(ids).toContain('gpt-5.1-codex-mini')
  })

  it('omits the pro tier until the chat path supports non-streaming responses', () => {
    expect(models.some((model) => model.modelId.endsWith('-pro'))).toBe(false)
    expect(getModelContextLength('chatgpt-pro', 'gpt-5.5-pro')).toBeUndefined()
  })

  it('omits models the Codex backend does not serve', () => {
    const ids = models.map((model) => model.modelId)

    expect(ids).not.toContain('gpt-4o')
    expect(ids).not.toContain('gpt-4.1')
    expect(ids).not.toContain('o3')
  })

  it('carries context windows so a saved provider sizes its conversation', () => {
    expect(getModelContextLength('chatgpt-pro', 'gpt-5.5')).toBe(1050000)
    expect(getModelContextLength('chatgpt-pro', 'gpt-5.1')).toBe(400000)
  })
})

describe('modelSupportsReasoning', () => {
  it('assumes a model the catalogue has never heard of reasons', () => {
    // Matches what buildChatRequestBody already tells the server about an
    // unknown model, and keeps the effort control on a pasted model id.
    expect(modelSupportsReasoning(undefined)).toBe(true)
  })

  it('takes the catalogue at its word for a known model', () => {
    expect(
      modelSupportsReasoning({
        modelId: 'x',
        contextLength: 1,
        supportsReasoning: false,
      }),
    ).toBe(false)
  })
})
