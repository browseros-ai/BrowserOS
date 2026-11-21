import { describe, expect, test } from 'bun:test'
import type { UIMessage } from 'ai'
import { getMessageSegments } from './getMessageSegments'

describe('getMessageSegments', () => {
  test('keeps provider-executed tool parts regardless of call id prefix', () => {
    const message = {
      id: 'message-1',
      role: 'assistant',
      parts: [
        {
          type: 'dynamic-tool',
          toolCallId: 'acpx-call-1',
          toolName: 'browser_navigate',
          state: 'output-available',
          input: { url: 'https://example.com' },
          output: { title: 'Example' },
        },
      ],
    } as unknown as UIMessage

    expect(getMessageSegments(message, true, false)).toEqual([
      {
        type: 'tool-batch',
        key: 'message-1-tools-acpx-call-1',
        tools: [
          {
            state: 'output-available',
            toolCallId: 'acpx-call-1',
            toolName: 'browser_navigate',
            input: { url: 'https://example.com' },
            output: { title: 'Example' },
            approval: undefined,
          },
        ],
      },
    ])
  })

  test('renders nothing for a suggest_schedule call, in any state', () => {
    const suggestion = JSON.stringify({
      type: 'schedule_suggestion',
      query: 'Summarize my inbox',
    })
    const message = {
      id: 'message-1',
      role: 'assistant',
      parts: [
        {
          type: 'tool-suggest_schedule',
          toolCallId: 'call-1',
          state: 'input-available',
          input: {},
        },
        {
          type: 'tool-suggest_schedule',
          toolCallId: 'call-2',
          state: 'output-available',
          input: {},
          output: { content: [{ type: 'text', text: suggestion }] },
        },
      ],
    } as unknown as UIMessage

    expect(getMessageSegments(message, true, false)).toEqual([])
  })
})
