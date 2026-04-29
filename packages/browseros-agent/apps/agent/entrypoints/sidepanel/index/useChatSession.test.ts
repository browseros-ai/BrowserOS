import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { ChatMode } from './chatTypes'
import type { SidepanelChatTarget } from './sidepanel-chat-targets'
import { buildSidepanelPreparedSendMessagesRequest } from './useChatSessionRequest'

const conversationId = '00000000-0000-4000-8000-000000000001'

describe('buildSidepanelPreparedSendMessagesRequest', () => {
  it('keeps LLM targets on the existing /chat request body', () => {
    const request = buildSidepanelPreparedSendMessagesRequest({
      agentServerUrl: 'http://127.0.0.1:5151',
      target: llmTarget,
      fallbackProvider,
      message: 'Summarize this page',
      ...commonRequestInput(),
    })

    expect(request.api).toBe('http://127.0.0.1:5151/chat')
    expect(request.body).toMatchObject({
      message: 'Summarize this page',
      conversationId,
      provider: 'browseros',
      providerType: 'browseros',
      providerName: 'BrowserOS',
      model: 'gpt-5',
      mode: 'agent',
      browserContext: {
        activeTab: { id: 10, url: 'https://example.com', title: 'Example' },
        enabledMcpServers: ['slack'],
      },
      userSystemPrompt: 'Be concise',
      userWorkingDir: '/tmp/work',
      previousConversation: [{ role: 'assistant', content: 'Prior answer' }],
      selectedText: 'selected text',
      selectedTextSource: {
        url: 'https://example.com',
        title: 'Example',
      },
    })
  })

  it('sends ACP targets to the sidepanel ACP route with explicit target fields', () => {
    const request = buildSidepanelPreparedSendMessagesRequest({
      agentServerUrl: 'http://127.0.0.1:5151',
      target: acpTarget,
      fallbackProvider,
      message: 'Inspect the current tab',
      approvalResponses: [
        { approvalId: 'approval-1', approved: true, reason: 'ok' },
      ],
      ...commonRequestInput(),
    })

    expect(request.api).toBe('http://127.0.0.1:5151/agents/sidepanel/chat')
    expect(request.body).toEqual({
      conversationId,
      adapter: 'codex',
      modelId: 'gpt-5.5',
      reasoningEffort: 'medium',
      message: 'Inspect the current tab',
      browserContext: {
        activeTab: { id: 10, url: 'https://example.com', title: 'Example' },
        enabledMcpServers: ['slack'],
      },
      userSystemPrompt: 'Be concise',
      userWorkingDir: '/tmp/work',
      selectedText: 'selected text',
      selectedTextSource: {
        url: 'https://example.com',
        title: 'Example',
      },
    })
  })

  it('keeps tool approval retry payloads scoped to LLM chat', () => {
    const request = buildSidepanelPreparedSendMessagesRequest({
      agentServerUrl: 'http://127.0.0.1:5151',
      target: llmTarget,
      fallbackProvider,
      approvalResponses: [
        { approvalId: 'approval-1', approved: false, reason: 'no' },
      ],
      ...commonRequestInput(),
    })

    expect(request.api).toBe('http://127.0.0.1:5151/chat')
    expect(request.body).toMatchObject({
      message: '',
      toolApprovalResponses: [
        { approvalId: 'approval-1', approved: false, reason: 'no' },
      ],
    })
  })
})

function commonRequestInput() {
  return {
    conversationId,
    mode: 'agent' as ChatMode,
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
    aclRules: [{ id: 'rule-1', sitePattern: '*://*/*', enabled: true }],
    selectedText: 'selected text',
    selectedTextSource: {
      url: 'https://example.com',
      title: 'Example',
    },
    toolApprovalConfig: { categories: { navigation: true } },
  }
}

const fallbackProvider: LlmProviderConfig = {
  id: 'browseros',
  type: 'browseros',
  name: 'BrowserOS',
  modelId: 'gpt-5',
  supportsImages: true,
  contextWindow: 128000,
  temperature: 0.7,
  createdAt: 1000,
  updatedAt: 1000,
}

const llmTarget: SidepanelChatTarget = {
  kind: 'llm',
  id: fallbackProvider.id,
  name: fallbackProvider.name,
  type: fallbackProvider.type,
  provider: fallbackProvider,
}

const acpTarget: SidepanelChatTarget = {
  kind: 'acp',
  id: 'acp:codex:gpt-5.5:medium',
  name: 'Codex GPT-5.5',
  type: 'acp',
  adapter: 'codex',
  adapterName: 'Codex',
  modelId: 'gpt-5.5',
  modelLabel: 'GPT-5.5',
  modelControl: 'best-effort',
  reasoningEffort: 'medium',
  reasoningEffortLabel: 'Medium',
}
