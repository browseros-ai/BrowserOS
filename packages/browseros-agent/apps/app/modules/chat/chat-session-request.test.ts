import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  buildSidepanelPreparedSendMessagesRequest,
  prepareSidepanelSendMessagesRequest,
} from './chat-session-request'
import type { SidepanelChatTarget } from './sidepanel-chat-targets'

const conversationId = '00000000-0000-4000-8000-000000000001'

describe('chat request preparation', () => {
  it('sends BrowserOS providers to the unified chat endpoint', () => {
    const request = buildSidepanelPreparedSendMessagesRequest({
      agentServerUrl: 'http://127.0.0.1:5151',
      target: llmTarget,
      fallbackProvider,
      message: 'Summarize this page',
      ...commonRequestInput(),
    })

    expect(request.api).toBe('http://127.0.0.1:5151/chat')
    expect(request.body).toMatchObject({
      target: { type: 'browseros', providerId: 'browseros' },
      provider: 'browseros',
      message: 'Summarize this page',
    })
  })

  it('sends ACP agents to the same endpoint without provider fields', () => {
    const request = buildSidepanelPreparedSendMessagesRequest({
      agentServerUrl: 'http://127.0.0.1:5151',
      target: acpTarget,
      fallbackProvider,
      message: 'Inspect the current tab',
      attachments: [
        {
          mediaType: 'image/png',
          data: 'data:image/png;base64,Zm9v',
        },
      ],
      ...commonRequestInput(),
    })

    expect(request.api).toBe('http://127.0.0.1:5151/chat')
    expect(request.body).toEqual({
      target: { type: 'codex', agentId: acpTarget.agentId },
      conversationId,
      message: 'Inspect the current tab',
      mode: 'agent',
      browserContext: commonRequestInput().browserContext,
      userSystemPrompt: 'Be concise',
      userWorkingDir: '/tmp/work',
      supportsImages: undefined,
      previousConversation: commonRequestInput().previousConversation,
      declinedApps: ['gmail'],
      selectedText: 'selected text',
      selectedTextSource: commonRequestInput().selectedTextSource,
      attachments: [
        {
          mediaType: 'image/png',
          data: 'data:image/png;base64,Zm9v',
        },
      ],
    })
    expect('provider' in request.body).toBe(false)
  })

  it('resolves the server URL for every send', async () => {
    let port = 9200
    const resolveAgentServerUrl = async () => `http://127.0.0.1:${port++}`

    const first = await prepareSidepanelSendMessagesRequest({
      resolveAgentServerUrl,
      target: llmTarget,
      fallbackProvider,
      ...commonRequestInput(),
    })
    const second = await prepareSidepanelSendMessagesRequest({
      resolveAgentServerUrl,
      target: llmTarget,
      fallbackProvider,
      ...commonRequestInput(),
    })

    expect(first.api).toBe('http://127.0.0.1:9200/chat')
    expect(second.api).toBe('http://127.0.0.1:9201/chat')
  })
})

function commonRequestInput() {
  return {
    conversationId,
    mode: 'agent' as const,
    browserContext: {
      activeTab: { id: 10, url: 'https://example.com', title: 'Example' },
      enabledMcpServers: ['slack'],
    },
    userSystemPrompt: 'Be concise',
    userWorkingDir: '/tmp/work',
    previousConversation: [
      { role: 'assistant' as const, content: 'Prior answer' },
    ],
    declinedApps: ['gmail'],
    selectedText: 'selected text',
    selectedTextSource: {
      url: 'https://example.com',
      title: 'Example',
    },
  }
}

const fallbackProvider: LlmProviderConfig = {
  id: 'browseros',
  type: 'browseros',
  name: 'BrowserOS',
  modelId: 'browseros-auto',
  supportsImages: true,
  contextWindow: 200000,
  temperature: 0.2,
  createdAt: 1,
  updatedAt: 1,
}

const llmTarget: SidepanelChatTarget = {
  kind: 'llm',
  id: fallbackProvider.id,
  name: fallbackProvider.name,
  type: fallbackProvider.type,
  provider: fallbackProvider,
}

const acpTarget: Extract<SidepanelChatTarget, { kind: 'acp' }> = {
  kind: 'acp',
  id: '00000000-0000-4000-8000-000000000002',
  name: 'Review bot',
  type: 'acp',
  agentId: '00000000-0000-4000-8000-000000000002',
  agentType: 'codex',
  adapterName: 'Codex',
  modelId: 'gpt-5.5',
  modelLabel: 'GPT-5.5',
  reasoningEffort: 'medium',
}
