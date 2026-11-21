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

  test('renders nothing for a hidden nudge call, in any state', () => {
    const output = (body: object) => ({
      content: [{ type: 'text', text: JSON.stringify(body) }],
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
          output: output({
            type: 'schedule_suggestion',
            query: 'Summarize my inbox',
          }),
        },
        {
          type: 'tool-suggest_app_connection',
          toolCallId: 'call-3',
          state: 'input-available',
          input: {},
        },
        {
          type: 'tool-suggest_app_connection',
          toolCallId: 'call-4',
          state: 'output-available',
          input: {},
          output: output({ type: 'app_connection', appName: 'Gmail' }),
        },
      ],
    } as unknown as UIMessage

    expect(getMessageSegments(message, true, false)).toEqual([])
  })

  test('joins the tools on either side of a hidden call into one batch', () => {
    const tool = (toolCallId: string, toolName: string) => ({
      type: `tool-${toolName}`,
      toolCallId,
      state: 'output-available',
      input: {},
      output: {},
    })
    const message = {
      id: 'message-1',
      role: 'assistant',
      parts: [
        tool('call-1', 'browser_navigate'),
        tool('call-2', 'suggest_app_connection'),
        tool('call-3', 'browser_click'),
      ],
    } as unknown as UIMessage

    const segments = getMessageSegments(message, true, false)

    expect(segments).toHaveLength(1)
    expect(segments[0]).toMatchObject({
      type: 'tool-batch',
      key: 'message-1-tools-call-1',
      tools: [
        { toolCallId: 'call-1', toolName: 'browser_navigate' },
        { toolCallId: 'call-3', toolName: 'browser_click' },
      ],
    })
  })
})
