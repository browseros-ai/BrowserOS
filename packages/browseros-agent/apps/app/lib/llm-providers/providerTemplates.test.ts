import { describe, expect, it } from 'bun:test'
import { providerTemplates } from './providerTemplates'

describe('providerTemplates', () => {
  it('uses the verified Codex default for new ChatGPT providers', () => {
    const template = providerTemplates.find(
      (provider) => provider.id === 'chatgpt-pro',
    )

    expect(template).toMatchObject({
      name: 'ChatGPT',
      defaultModelId: 'gpt-6.1-sol',
      contextWindow: 500000,
    })
  })
})
