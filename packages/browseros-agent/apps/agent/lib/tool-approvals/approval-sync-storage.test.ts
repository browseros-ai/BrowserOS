import { describe, expect, it } from 'bun:test'
import type { UIMessage } from 'ai'
import {
  extractPendingApprovals,
  queueApprovalResponse,
  removeApprovalResponsesById,
  removePendingApprovalsById,
  replacePendingApprovalsForConversation,
} from './approval-sync-helpers'

describe('approval sync storage helpers', () => {
  it('extracts pending approvals from assistant tool parts', () => {
    const messages = [
      {
        id: 'assistant-1',
        role: 'assistant',
        parts: [
          {
            type: 'tool-click',
            state: 'approval-requested',
            toolCallId: 'tool-1',
            input: { selector: '#buy-now' },
            approval: { id: 'approval-1' },
          },
        ],
      },
    ] as UIMessage[]

    expect(extractPendingApprovals(messages, 'conversation-1', 123)).toEqual([
      {
        approvalId: 'approval-1',
        toolCallId: 'tool-1',
        toolName: 'click',
        input: { selector: '#buy-now' },
        conversationId: 'conversation-1',
        timestamp: 123,
      },
    ])
  })

  it('replaces pending approvals for one conversation without clearing others', () => {
    const existing = [
      {
        approvalId: 'approval-a',
        toolCallId: 'tool-a',
        toolName: 'click',
        input: {},
        conversationId: 'conversation-a',
        timestamp: 1,
      },
      {
        approvalId: 'approval-b',
        toolCallId: 'tool-b',
        toolName: 'navigate_page',
        input: {},
        conversationId: 'conversation-b',
        timestamp: 2,
      },
    ]

    expect(
      replacePendingApprovalsForConversation(existing, 'conversation-a', []),
    ).toEqual([existing[1]])
  })

  it('queues and removes approval responses by approval id', () => {
    const queued = queueApprovalResponse(
      [
        {
          approvalId: 'approval-a',
          approved: true,
          timestamp: 1,
        },
      ],
      {
        approvalId: 'approval-b',
        approved: false,
        timestamp: 2,
      },
    )

    expect(queued).toEqual([
      {
        approvalId: 'approval-a',
        approved: true,
        timestamp: 1,
      },
      {
        approvalId: 'approval-b',
        approved: false,
        timestamp: 2,
      },
    ])

    expect(removeApprovalResponsesById(queued, ['approval-a'])).toEqual([
      {
        approvalId: 'approval-b',
        approved: false,
        timestamp: 2,
      },
    ])
  })

  it('removes only handled pending approvals', () => {
    const pending = [
      {
        approvalId: 'approval-a',
        toolCallId: 'tool-a',
        toolName: 'click',
        input: {},
        conversationId: 'conversation-a',
        timestamp: 1,
      },
      {
        approvalId: 'approval-b',
        toolCallId: 'tool-b',
        toolName: 'fill',
        input: {},
        conversationId: 'conversation-b',
        timestamp: 2,
      },
    ]

    expect(removePendingApprovalsById(pending, ['approval-b'])).toEqual([
      pending[0],
    ])
  })
})
