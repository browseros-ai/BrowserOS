import { describe, expect, it } from 'bun:test'
import {
  getModelContextLength,
  getModelsForProvider,
  modelSupportsReasoning,
} from './models'

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
