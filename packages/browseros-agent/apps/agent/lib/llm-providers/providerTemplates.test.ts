import { describe, expect, it } from 'bun:test'
import { providerTemplates } from './providerTemplates'

describe('providerTemplates', () => {
  it('uses GPT-5.5 for new ChatGPT Plus/Pro providers', () => {
    const template = providerTemplates.find(
      (provider) => provider.id === 'chatgpt-pro',
    )

    expect(template).toMatchObject({
      defaultModelId: 'gpt-5.5',
      contextWindow: 1050000,
    })
  })
})
