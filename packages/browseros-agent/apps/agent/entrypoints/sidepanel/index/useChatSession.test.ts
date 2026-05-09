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

  it('sends created-agent targets to the agent-id sidepanel route', () => {
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

    expect(request.api).toBe(
      'http://127.0.0.1:5151/agents/agent-codex/sidepanel/chat',
    )
    expect(request.body).toEqual({
      conversationId,
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

  it('forwards attached context snippets to chat endpoints', () => {
    const contextAttachments = [
      {
        id: 'file:README.md',
        kind: 'file' as const,
        title: 'README.md',
        source: 'README.md',
        content: '# Project',
      },
      {
        id: 'memory:CORE.md',
        kind: 'memory' as const,
        title: 'Core memory',
        source: 'CORE.md',
        content: 'User prefers concise answers.',
      },
    ]

    const llmRequest = buildSidepanelPreparedSendMessagesRequest({
      agentServerUrl: 'http://127.0.0.1:5151',
      target: llmTarget,
      fallbackProvider,
      message: 'Use this context',
      ...commonRequestInput(),
      contextAttachments,
    })

    expect(llmRequest.body.contextAttachments).toEqual([
      {
        kind: 'file',
        title: 'README.md',
        source: 'README.md',
        content: '# Project',
      },
      {
        kind: 'memory',
        title: 'Core memory',
        source: 'CORE.md',
        content: 'User prefers concise answers.',
      },
    ])

    const acpRequest = buildSidepanelPreparedSendMessagesRequest({
      agentServerUrl: 'http://127.0.0.1:5151',
      target: acpTarget,
      fallbackProvider,
      message: 'Use this context',
      ...commonRequestInput(),
      contextAttachments,
    })

    expect(acpRequest.body).toMatchObject({ contextAttachments })
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
  id: 'agent-codex',
  name: 'Review bot',
  type: 'acp',
  agentId: 'agent-codex',
  adapter: 'codex',
  adapterName: 'Codex',
  modelId: 'gpt-5.5',
  modelLabel: 'GPT-5.5',
  modelControl: 'best-effort',
  reasoningEffort: 'medium',
  reasoningEffortLabel: 'Medium',
}
