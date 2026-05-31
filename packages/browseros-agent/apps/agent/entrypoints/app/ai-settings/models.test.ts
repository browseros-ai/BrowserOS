import { describe, expect, it } from 'bun:test'
import { getModelContextLength, getModelsForProvider } from './models'

describe('ChatGPT Plus/Pro models', () => {
  it('offers GPT-5.5 as the default first choice', () => {
    const models = getModelsForProvider('chatgpt-pro')

    expect(models[0]).toEqual({
      modelId: 'gpt-5.5',
      contextLength: 1050000,
    })
    expect(getModelContextLength('chatgpt-pro', 'gpt-5.5')).toBe(1050000)
  })
})
