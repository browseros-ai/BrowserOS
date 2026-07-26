import { describe, expect, it } from 'bun:test'
import { getModelContextLength, getModelsForProvider } from './models'

describe('ChatGPT models', () => {
  it('offers GPT-5.5 as the default first choice', () => {
    const models = getModelsForProvider('chatgpt-pro')

    expect(models[0]).toEqual({
      modelId: 'gpt-5.5',
      contextLength: 1050000,
    })
    expect(getModelContextLength('chatgpt-pro', 'gpt-5.5')).toBe(1050000)
  })

  it('includes current GPT-5.4 options', () => {
    expect(getModelContextLength('chatgpt-pro', 'gpt-5.4')).toBe(1050000)
    expect(getModelContextLength('chatgpt-pro', 'gpt-5.4-mini')).toBe(400000)
    expect(getModelContextLength('chatgpt-pro', 'gpt-5.4-nano')).toBe(400000)
  })

  it('omits GPT-5.5 Pro until the chat path supports non-streaming responses', () => {
    const models = getModelsForProvider('chatgpt-pro')

    expect(models.some((model) => model.modelId === 'gpt-5.5-pro')).toBe(false)
    expect(getModelContextLength('chatgpt-pro', 'gpt-5.5-pro')).toBeUndefined()
  })

  it('uses current context windows for older GPT-5 frontier models', () => {
    expect(getModelContextLength('chatgpt-pro', 'gpt-5.2')).toBe(400000)
    expect(getModelContextLength('chatgpt-pro', 'gpt-5.1')).toBe(400000)
  })
})
