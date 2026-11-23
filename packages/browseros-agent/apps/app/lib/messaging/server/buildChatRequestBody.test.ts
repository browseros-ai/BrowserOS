import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { buildChatRequestBody } from './buildChatRequestBody'

const provider: LlmProviderConfig = {
  id: 'openai-1',
  type: 'openai',
  name: 'OpenAI',
  modelId: 'gpt-5',
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
  })

  it.each([true, undefined])(
    'uses local history with persist=%s',
    (persist) => {
      const body = buildChatRequestBody({
        conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
        persist,
      })

      expect(body.persist).toBe(persist)
      expect(body.historyMode).toBe('local')
      expect(body.previousConversation).toBeUndefined()
    },
  )

  it('sends client history for an incognito turn', () => {
    const body = buildChatRequestBody({
      conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
      persist: false,
      previousConversation: [{ role: 'assistant', content: 'Prior answer' }],
    })

    expect(body).toMatchObject({
      persist: false,
      historyMode: 'cloud',
      previousConversation: [{ role: 'assistant', content: 'Prior answer' }],
    })
  })

  // The provider is named and nothing more. Its model, endpoint and
  // credentials are resolved from the id server side, so an api key and an aws
  // secret no longer cross the wire on every message.
  it('sends no provider configuration or credentials', () => {
    const body = buildChatRequestBody({
      conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
      provider: {
        ...provider,
        id: 'bedrock-1',
        type: 'bedrock',
        apiKey: 'sk-secret',
        accessKeyId: 'AKIA',
        secretAccessKey: 'aws-secret',
        sessionToken: 'token',
        baseUrl: 'https://example.com',
      },
    })

    for (const field of [
      'apiKey',
      'accessKeyId',
      'secretAccessKey',
      'sessionToken',
      'baseUrl',
      'model',
      'provider',
      'providerId',
      'providerType',
      'providerName',
      'temperature',
      'contextWindowSize',
      'region',
      'resourceName',
    ]) {
      expect(field in body).toBe(false)
    }
    expect(JSON.stringify(body)).not.toContain('aws-secret')
    expect(JSON.stringify(body)).not.toContain('sk-secret')
  })

  it('preserves browser context and chat metadata', () => {
    const body = buildChatRequestBody({
      conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
      provider,
      mode: 'agent',
      browserContext: {
        windowId: 2,
        activeTab: { id: 10, url: 'https://amazon.com', title: 'Amazon' },
      },
      userSystemPrompt: 'Stay in the current tab.',
    })

    expect(body.browserContext).toMatchObject({
      windowId: 2,
      activeTab: { id: 10, url: 'https://amazon.com' },
    })
    expect(body.userSystemPrompt).toBe('Stay in the current tab.')
  })

  it('drops tabs the server cannot address', () => {
    const body = buildChatRequestBody({
      conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
      browserContext: {
        windowId: 2,
        activeTab: { title: 'Opening' },
        selectedTabs: [{ title: 'Opening' }, { id: 0, title: 'Selected' }],
      },
    })

    expect(body.browserContext?.activeTab).toBeUndefined()
    expect(body.browserContext?.selectedTabs).toEqual([
      { id: 0, title: 'Selected' },
    ])
    expect(body).not.toHaveProperty('supportsImages')
  })
})
