import { describe, expect, it } from 'bun:test'
import type { UIMessage } from 'ai'
import { OMITTED_IMAGE_TEXT, stripImageToolOutputs } from './tool-output-strip'

type UIPart = UIMessage['parts'][number]

function textPart(text: string): UIPart {
  return { type: 'text', text } as UIPart
}

function screenshotToolPart(options?: {
  data?: string
  extraContent?: Array<Record<string, unknown>>
  metadata?: Record<string, unknown>
}): UIPart {
  return {
    type: 'tool-screenshot',
    toolCallId: 'call-1',
    state: 'output-available',
    input: {},
    output: {
      content: [
        {
          type: 'image',
          data: options?.data ?? 'BASE64_SCREENSHOT_DATA',
          mimeType: 'image/png',
        },
        { type: 'text', text: 'took screenshot' },
        ...(options?.extraContent ?? []),
      ],
      isError: false,
      ...(options?.metadata ? { metadata: options.metadata } : {}),
    },
  } as UIPart
}

function assistantMessage(id: string, parts: UIPart[]): UIMessage {
  return { id, role: 'assistant', parts }
}

describe('stripImageToolOutputs', () => {
  it('replaces base64 image blocks with a text marker and keeps text blocks', () => {
    const messages = [assistantMessage('a', [screenshotToolPart()])]

    const result = stripImageToolOutputs(messages)
    const output = (result[0].parts[0] as { output: { content: unknown[] } })
      .output

    expect(output.content).toEqual([
      { type: 'text', text: OMITTED_IMAGE_TEXT },
      { type: 'text', text: 'took screenshot' },
    ])
  })

  it('preserves output metadata such as the active tab id', () => {
    const messages = [
      assistantMessage('a', [screenshotToolPart({ metadata: { tabId: 42 } })]),
    ]

    const result = stripImageToolOutputs(messages)
    const output = (result[0].parts[0] as { output: { metadata?: unknown } })
      .output

    expect(output.metadata).toEqual({ tabId: 42 })
  })

  it('returns the same array reference when there is nothing to strip', () => {
    const messages = [
      assistantMessage('a', [textPart('hello'), textPart('world')]),
    ]

    expect(stripImageToolOutputs(messages)).toBe(messages)
  })

  it('leaves non-tool parts untouched', () => {
    const messages = [
      assistantMessage('a', [
        { type: 'reasoning', text: 'thinking' } as UIPart,
        textPart('answer'),
      ]),
    ]

    expect(stripImageToolOutputs(messages)).toBe(messages)
  })

  it('keeps the last message intact when keepLastMessage is set', () => {
    const older = assistantMessage('older', [screenshotToolPart()])
    const latest = assistantMessage('latest', [screenshotToolPart()])

    const result = stripImageToolOutputs([older, latest], {
      keepLastMessage: true,
    })

    const olderOutput = (
      result[0].parts[0] as { output: { content: unknown[] } }
    ).output
    const latestOutput = (
      result[1].parts[0] as { output: { content: unknown[] } }
    ).output

    expect(olderOutput.content[0]).toEqual({
      type: 'text',
      text: OMITTED_IMAGE_TEXT,
    })
    expect(latestOutput.content[0]).toEqual({
      type: 'image',
      data: 'BASE64_SCREENSHOT_DATA',
      mimeType: 'image/png',
    })
    expect(result[1]).toBe(latest)
  })

  it('does not throw on tool parts with non-object or missing output', () => {
    const messages = [
      assistantMessage('a', [
        {
          type: 'tool-run',
          toolCallId: 'c',
          state: 'input-available',
          input: {},
        } as UIPart,
        {
          type: 'dynamic-tool',
          toolName: 'run',
          toolCallId: 'd',
          state: 'output-available',
          input: {},
          output: 'plain string',
        } as UIPart,
      ]),
    ]

    expect(() => stripImageToolOutputs(messages)).not.toThrow()
    expect(stripImageToolOutputs(messages)).toBe(messages)
  })
})
