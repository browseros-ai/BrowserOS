import { describe, expect, it } from 'bun:test'
import { providerTemplates } from './providerTemplates'

describe('providerTemplates', () => {
  it('uses ChatGPT as the display name for new ChatGPT providers', () => {
    const template = providerTemplates.find(
      (provider) => provider.id === 'chatgpt-pro',
    )

    expect(template).toMatchObject({
      name: 'ChatGPT',
      defaultModelId: 'gpt-5.5',
      contextWindow: 1050000,
    })
  })
})
