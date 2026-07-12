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

  it('defaults MiniMax to M3 on the international endpoint with a 1M context', () => {
    const template = providerTemplates.find(
      (provider) => provider.id === 'minimax',
    )

    expect(template).toMatchObject({
      name: 'MiniMax',
      defaultBaseUrl: 'https://api.minimax.io/v1',
      defaultModelId: 'MiniMax-M3',
      contextWindow: 1000000,
      setupGuideUrl:
        'https://platform.minimax.io/docs/api-reference/text-openai-api',
    })
  })
})
