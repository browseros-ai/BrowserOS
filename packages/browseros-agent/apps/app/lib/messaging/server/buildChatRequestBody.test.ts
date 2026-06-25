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
  it('preserves browser context and chat metadata', () => {
    const body = buildChatRequestBody({
      conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
      provider,
      mode: 'agent',
      browserContext: {
        windowId: 2,
        activeTab: {
          id: 10,
          url: 'https://amazon.com',
          title: 'Amazon',
        },
        enabledMcpServers: ['slack'],
      },
      userSystemPrompt: 'Stay in the current tab.',
      declinedApps: ['gmail'],
    })

    expect(body.browserContext).toEqual({
      windowId: 2,
      activeTab: {
        id: 10,
        url: 'https://amazon.com',
        title: 'Amazon',
      },
      enabledMcpServers: ['slack'],
    })
    expect(body.userSystemPrompt).toBe('Stay in the current tab.')
    expect(body.declinedApps).toEqual(['gmail'])
  })

  it('forwards the provider id so the server can scope per-provider state', () => {
    const body = buildChatRequestBody({
      conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
      provider: { ...provider, id: 'uuid-opus-high' },
    })
    expect(body.providerId).toBe('uuid-opus-high')
  })

  it('forwards every ACP field so the chat path can reach a custom agent', () => {
    const acpProvider: LlmProviderConfig = {
      ...provider,
      id: 'uuid-claude-opus',
      type: 'claude-code',
      name: 'Claude Opus',
      acpAgentId: 'claude',
      acpCommand: undefined,
      acpFixedWorkspacePath: '/home/user/agents/claude-opus',
    }
    const body = buildChatRequestBody({
      conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
      provider: acpProvider,
    })
    expect(body.providerId).toBe('uuid-claude-opus')
    expect(body.acpAgentId).toBe('claude')
    expect(body.acpCommand).toBeUndefined()
    expect(body.acpFixedWorkspacePath).toBe('/home/user/agents/claude-opus')
  })

  it('leaves ACP fields undefined for non-ACP providers', () => {
    const body = buildChatRequestBody({
      conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
      provider,
    })
    expect(body.acpAgentId).toBeUndefined()
    expect(body.acpCommand).toBeUndefined()
    expect(body.acpFixedWorkspacePath).toBeUndefined()
  })
})
