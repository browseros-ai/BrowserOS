import { describe, expect, test } from 'bun:test'
import type { UIMessage } from 'ai'
import {
  TOOL_NOT_RUN,
  TOOL_STOPPED,
} from '@/lib/messaging/server/tool-output-status'
import stopMidCall from '../../../../../crates/browseros-server/tests/fixtures/ui-stream/stop-mid-call.message.json'
import stopAfterFirst from '../../../../../crates/browseros-server/tests/fixtures/ui-stream/two-calls-stop-after-first.message.json'
import {
  getMessageSegments,
  type ToolInvocationInfo,
} from './getMessageSegments'

describe('getMessageSegments', () => {
  test('skipped status labels match the Rust transcript goldens', () => {
    const outputError = ({
      parts,
    }: typeof stopMidCall | typeof stopAfterFirst) =>
      parts.find((part) => part.state === 'output-error')?.errorText

    expect(outputError(stopMidCall)).toBe(TOOL_STOPPED)
    expect(outputError(stopAfterFirst)).toBe(TOOL_NOT_RUN)
  })

  test.each([
    [TOOL_STOPPED, TOOL_STOPPED],
    [TOOL_NOT_RUN, TOOL_NOT_RUN],
    ['Interrupted', undefined],
    ['Script failed', undefined],
  ] satisfies [string, ToolInvocationInfo['skippedStatus']][])(
    'projects %s into the batch',
    (errorText, skippedStatus) => {
      const message: UIMessage = {
        id: 'assistant-1',
        role: 'assistant',
        parts: [
          {
            type: 'tool-run',
            toolCallId: 'call-1',
            state: 'output-error',
            input: undefined,
            errorText,
          },
        ],
      }
      const [batch] = getMessageSegments(message, true, false)
      expect(batch?.type).toBe('tool-batch')
      if (batch?.type === 'tool-batch') {
        expect(batch.tools[0].skippedStatus).toBe(skippedStatus)
      }
    },
  )

  test('keeps provider-executed tool parts regardless of call id prefix', () => {
    const message: UIMessage = {
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
    }

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
            skippedStatus: undefined,
          },
        ],
      },
    ])
  })
})
