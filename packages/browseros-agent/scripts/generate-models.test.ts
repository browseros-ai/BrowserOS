import { describe, expect, test } from 'bun:test'
import modelsDevData from '../apps/app/lib/llm-providers/models-dev-data.json'
import {
  formatModelsData,
  generateModelsData,
  type ModelsDevModel,
  type ModelsDevProvider,
  type OutputProvider,
} from './generate-models'

const REQUIRED_PROVIDER_IDS = [
  'anthropic',
  'openai',
  'google',
  'openrouter',
  'azure',
  'bedrock',
  'lmstudio',
  'moonshot',
  'github-copilot',
]

const NON_CHAT_MODEL_CLASS_PATTERN =
  /embedding|image|audio|tts|transcribe|whisper|moderation/i

function model(overrides: Partial<ModelsDevModel>): ModelsDevModel {
  return {
    id: 'model-a',
    name: 'Model A',
    attachment: false,
    reasoning: false,
    tool_call: true,
    modalities: { input: ['text'], output: ['text'] },
    limit: { context: 128000, output: 8192 },
    release_date: '2026-01-01',
    last_updated: '2026-01-01',
    ...overrides,
  }
}

function provider(models: Record<string, ModelsDevModel>): ModelsDevProvider {
  return {
    id: 'source-provider',
    name: 'Source Provider',
    npm: '@ai-sdk/source-provider',
    doc: 'https://example.com/docs',
    env: ['SOURCE_API_KEY'],
    models,
  }
}

describe('generateModelsData', () => {
  test('maps providers and omits deprecated models', () => {
    const output = generateModelsData(
      {
        'source-provider': provider({
          current: model({
            id: 'current-model',
            name: 'Current Model',
            attachment: true,
            reasoning: true,
            cost: { input: 1, output: 2 },
          }),
          deprecated: model({
            id: 'deprecated-model',
            status: 'deprecated',
          }),
        }),
      },
      { 'source-provider': 'browseros-provider' },
    )

    expect(Object.keys(output)).toEqual(['browseros-provider'])
    expect(output['browseros-provider']).toEqual({
      name: 'Source Provider',
      doc: 'https://example.com/docs',
      models: [
        {
          id: 'current-model',
          name: 'Current Model',
          contextWindow: 128000,
          maxOutput: 8192,
          supportsImages: true,
          supportsReasoning: true,
          supportsToolCall: true,
          inputCost: 1,
          outputCost: 2,
        },
      ],
    })
  })

  test('omits non-chat models', () => {
    const output = generateModelsData(
      {
        'source-provider': provider({
          imageOnly: model({
            id: 'image-only',
            modalities: { input: ['text'], output: ['image'] },
            limit: { context: 0, output: 0 },
          }),
          imageTextOutput: model({
            id: 'gemini-3-pro-image-preview',
            name: 'Nano Banana Pro',
            modalities: { input: ['text', 'image'], output: ['text', 'image'] },
          }),
          audioTextOutput: model({
            id: 'openai/gpt-audio',
            name: 'GPT Audio',
            modalities: { input: ['text', 'audio'], output: ['text', 'audio'] },
          }),
          embeddingFamily: model({
            id: 'text-embedding-3-large',
            family: 'text-embedding',
          }),
          missingLimits: model({
            id: 'missing-limits',
            modalities: { input: ['text'], output: ['text'] },
            limit: { context: 0, output: 8192 },
          }),
          noTextInput: model({
            id: 'non-text-input',
            modalities: { input: ['image'], output: ['text'] },
          }),
          chat: model({
            id: 'chat-model',
            modalities: { input: ['text', 'image'], output: ['text'] },
          }),
        }),
      },
      { 'source-provider': 'browseros-provider' },
    )

    expect(output['browseros-provider']?.models.map((m) => m.id)).toEqual([
      'chat-model',
    ])
  })

  test('sorts models by last update then id', () => {
    const output = generateModelsData(
      {
        'source-provider': provider({
          b: model({ id: 'b-model', last_updated: '2026-01-01' }),
          a: model({ id: 'a-model', last_updated: '2026-01-01' }),
          c: model({ id: 'c-model', last_updated: '2026-02-01' }),
        }),
      },
      { 'source-provider': 'browseros-provider' },
    )

    expect(output['browseros-provider']?.models.map((m) => m.id)).toEqual([
      'c-model',
      'a-model',
      'b-model',
    ])
  })

  test('rejects duplicate transformed model ids', () => {
    expect(() =>
      generateModelsData(
        {
          'source-provider': provider({
            first: model({ id: 'duplicate-model' }),
            second: model({ id: 'duplicate-model' }),
          }),
        },
        { 'source-provider': 'browseros-provider' },
      ),
    ).toThrow('Duplicate model id for browseros-provider: duplicate-model')
  })

  test('rejects missing required providers', () => {
    expect(() =>
      generateModelsData({}, { 'source-provider': 'browseros-provider' }),
    ).toThrow('Provider not found in models.dev: source-provider')
  })

  test('formats generated JSON with a trailing newline', () => {
    const output = {
      'browseros-provider': {
        name: 'Source Provider',
        doc: 'https://example.com/docs',
        models: [],
      },
    }

    const json = formatModelsData(output)

    expect(json.endsWith('\n')).toBe(true)
    expect(JSON.parse(json)).toEqual(output)
  })

  test('checked-in snapshot has required chat providers and models', () => {
    const data = modelsDevData as Record<string, OutputProvider>

    expect(Object.keys(data)).toEqual(REQUIRED_PROVIDER_IDS)

    for (const [providerId, provider] of Object.entries(data)) {
      expect(provider.models.length).toBeGreaterThan(0)

      const ids = new Set<string>()
      for (const model of provider.models) {
        expect(ids.has(model.id)).toBe(false)
        ids.add(model.id)

        expect(model.id).toBeTruthy()
        expect(model.name).toBeTruthy()
        expect(model.id).not.toMatch(NON_CHAT_MODEL_CLASS_PATTERN)
        expect(model.name).not.toMatch(NON_CHAT_MODEL_CLASS_PATTERN)
        expect(model.contextWindow).toBeGreaterThan(0)
        expect(model.maxOutput).toBeGreaterThan(0)
        expect(typeof model.supportsImages).toBe('boolean')
        expect(typeof model.supportsReasoning).toBe('boolean')
        expect(typeof model.supportsToolCall).toBe('boolean')
      }

      expect(ids.size).toBe(provider.models.length)
      expect(REQUIRED_PROVIDER_IDS).toContain(providerId)
    }
  })
})
