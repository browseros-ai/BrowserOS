import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { ToolApprovalConfig } from '@/lib/tool-approvals/types'
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
  it('preserves approval config and browser context on approval resumes', () => {
    const toolApprovalConfig: ToolApprovalConfig = {
      categories: {
        input: true,
        navigation: true,
        observation: true,
        screenshots: true,
        scripts: true,
        'data-modification': true,
        assistant: true,
      },
    }

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
      toolApprovalConfig,
      toolApprovalResponses: [
        {
          approvalId: 'approval-1',
          approved: true,
        },
      ],
    })

    expect(body.toolApprovalConfig).toEqual(toolApprovalConfig)
    expect(body.browserContext).toEqual({
      windowId: 2,
      activeTab: {
        id: 10,
        url: 'https://amazon.com',
        title: 'Amazon',
      },
      enabledMcpServers: ['slack'],
    })
    expect(body.toolApprovalResponses).toEqual([
      {
        approvalId: 'approval-1',
        approved: true,
      },
    ])
  })

  it('omits empty approval configs from requests', () => {
    const body = buildChatRequestBody({
      conversationId: '6ff46e3b-e45a-40a4-9157-ca520e800f43',
      provider,
      toolApprovalConfig: {
        categories: {},
      },
    })

    expect(body.toolApprovalConfig).toBeUndefined()
  })
})
