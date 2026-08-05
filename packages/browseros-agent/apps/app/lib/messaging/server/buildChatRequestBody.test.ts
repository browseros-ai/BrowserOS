import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { buildChatRequestBody } from './buildChatRequestBody'

const provider: LlmProviderConfig = {
  id: 'browseros',
  type: 'browseros',
  name: 'BrowserOS',
  modelId: 'browseros-auto',
  supportsImages: true,
  contextWindow: 200000,
  temperature: 0,
  createdAt: 0,
  updatedAt: 0,
}

describe('buildChatRequestBody', () => {
  it('creates an explicit BrowserOS target', () => {
    const body = buildChatRequestBody({
      conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
      provider: { ...provider, id: 'provider-1' },
    })

    expect(body.target).toEqual({
      type: 'browseros',
      providerId: 'provider-1',
    })
    expect(body.providerId).toBe('provider-1')
  })

  it('preserves browser context and chat metadata', () => {
    const body = buildChatRequestBody({
      conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
      provider,
      mode: 'agent',
      browserContext: {
        windowId: 2,
        activeTab: { id: 10, url: 'https://amazon.com', title: 'Amazon' },
        enabledMcpServers: ['slack'],
      },
      userSystemPrompt: 'Stay in the current tab.',
      declinedApps: ['gmail'],
    })

    expect(body.browserContext).toMatchObject({
      windowId: 2,
      activeTab: { id: 10, url: 'https://amazon.com' },
      enabledMcpServers: ['slack'],
    })
    expect(body.userSystemPrompt).toBe('Stay in the current tab.')
    expect(body.declinedApps).toEqual(['gmail'])
  })
})
