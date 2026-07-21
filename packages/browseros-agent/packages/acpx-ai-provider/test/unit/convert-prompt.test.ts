import { describe, expect, test } from 'bun:test'
import type {
  LanguageModelV2CallOptions,
  LanguageModelV2Prompt,
  LanguageModelV2ToolResultOutput,
} from '@ai-sdk/provider'
import { convertPrompt } from '../../src/convert-prompt.ts'

function call(
  prompt: LanguageModelV2Prompt,
  opts: {
    mode?: 'fresh' | 'continuation'
    responseFormat?: LanguageModelV2CallOptions['responseFormat']
  } = {},
) {
  return convertPrompt({
    prompt,
    mode: opts.mode ?? 'fresh',
    responseFormat: opts.responseFormat,
  })
}

describe('role serialization', () => {
  test('system messages are prefixed with "System: "', () => {
    const out = call([{ role: 'system', content: 'be terse' }])
    expect(out.text).toBe('System: be terse')
    expect(out.attachments).toEqual([])
  })

  test('user text part gets "User: " prefix', () => {
    const out = call([
      { role: 'user', content: [{ type: 'text', text: 'hi' }] },
    ])
    expect(out.text).toBe('User: hi')
  })

  test('assistant text part gets "Assistant: " prefix', () => {
    const out = call([
      { role: 'assistant', content: [{ type: 'text', text: 'hello' }] },
    ])
    expect(out.text).toBe('Assistant: hello')
  })

  test('tool-result message gets "Tool: " prefix', () => {
    const out = call([
      {
        role: 'tool',
        content: [
          {
            type: 'tool-result',
            toolCallId: 't1',
            toolName: 'greet',
            output: { type: 'text', value: 'hello world' },
          },
        ],
      },
    ])
    expect(out.text).toBe('Tool: [Tool result (t1): hello world]')
  })

  test('multiple text parts are joined with a single space', () => {
    const out = call([
      {
        role: 'user',
        content: [
          { type: 'text', text: 'foo' },
          { type: 'text', text: 'bar' },
        ],
      },
    ])
    expect(out.text).toBe('User: foo bar')
  })

  test('multi-message history is joined with newlines, role prefixes per message', () => {
    const out = call([
      { role: 'system', content: 'sys' },
      { role: 'user', content: [{ type: 'text', text: 'hi' }] },
      { role: 'assistant', content: [{ type: 'text', text: 'hello' }] },
    ])
    expect(out.text).toBe('System: sys\nUser: hi\nAssistant: hello')
  })

  test('messages with only file parts emit no text line', () => {
    const out = call([
      {
        role: 'user',
        content: [{ type: 'file', mediaType: 'image/png', data: 'Zm9v' }],
      },
    ])
    expect(out.text).toBe('')
    expect(out.attachments).toEqual([{ mediaType: 'image/png', data: 'Zm9v' }])
  })
})

describe('assistant-only parts', () => {
  test('reasoning parts are wrapped as [Reasoning: ...]', () => {
    const out = call([
      {
        role: 'assistant',
        content: [{ type: 'reasoning', text: 'thinking' }],
      },
    ])
    expect(out.text).toBe('Assistant: [Reasoning: thinking]')
  })

  test('tool-call parts serialize name and JSON args', () => {
    const out = call([
      {
        role: 'assistant',
        content: [
          {
            type: 'tool-call',
            toolCallId: 't1',
            toolName: 'add',
            input: { a: 1, b: 2 },
          },
        ],
      },
    ])
    expect(out.text).toBe('Assistant: [Tool call: add({"a":1,"b":2})]')
  })
})

describe('tool-result output formatting', () => {
  function toolResult(output: LanguageModelV2ToolResultOutput) {
    return call([
      {
        role: 'tool',
        content: [
          {
            type: 'tool-result',
            toolCallId: 't1',
            toolName: 'greet',
            output,
          },
        ],
      },
    ])
  }

  test('text output is rendered verbatim', () => {
    expect(toolResult({ type: 'text', value: 'hello' }).text).toBe(
      'Tool: [Tool result (t1): hello]',
    )
  })

  test('error-text output is rendered verbatim', () => {
    expect(toolResult({ type: 'error-text', value: 'boom' }).text).toBe(
      'Tool: [Tool result (t1): boom]',
    )
  })

  test('json output is JSON-stringified', () => {
    expect(toolResult({ type: 'json', value: { ok: true } }).text).toBe(
      'Tool: [Tool result (t1): {"ok":true}]',
    )
  })

  test('error-json output is JSON-stringified', () => {
    expect(toolResult({ type: 'error-json', value: { code: 1 } }).text).toBe(
      'Tool: [Tool result (t1): {"code":1}]',
    )
  })

  test('content output joins text + media descriptors', () => {
    const out = toolResult({
      type: 'content',
      value: [
        { type: 'text', text: 'see this:' },
        { type: 'media', data: 'Zm9v', mediaType: 'image/png' },
      ],
    })
    expect(out.text).toBe(
      'Tool: [Tool result (t1): see this: <media:image/png>]',
    )
  })
})

describe('file attachments', () => {
  test('extracts base64 from data: URLs', () => {
    const out = call([
      {
        role: 'user',
        content: [
          { type: 'text', text: 'see' },
          {
            type: 'file',
            mediaType: 'image/png',
            data: 'data:image/png;base64,Zm9v',
          },
        ],
      },
    ])
    expect(out.text).toBe('User: see')
    expect(out.attachments).toEqual([{ mediaType: 'image/png', data: 'Zm9v' }])
  })

  test('keeps plain base64 string as-is', () => {
    const out = call([
      {
        role: 'user',
        content: [{ type: 'file', mediaType: 'image/png', data: 'Zm9v' }],
      },
    ])
    expect(out.attachments).toEqual([{ mediaType: 'image/png', data: 'Zm9v' }])
  })

  test('encodes Uint8Array as base64', () => {
    const out = call([
      {
        role: 'user',
        content: [
          {
            type: 'file',
            mediaType: 'image/png',
            data: new Uint8Array([102, 111, 111]),
          },
        ],
      },
    ])
    expect(out.attachments).toEqual([{ mediaType: 'image/png', data: 'Zm9v' }])
  })

  test('throws on URL data with an actionable message', () => {
    expect(() =>
      call([
        {
          role: 'user',
          content: [
            {
              type: 'file',
              mediaType: 'image/png',
              data: new URL('https://example.com/img.png'),
            },
          ],
        },
      ]),
    ).toThrow(/inline the file as base64 or Uint8Array/)
  })

  test('preserves attachment order across multiple files and messages', () => {
    const out = call([
      {
        role: 'user',
        content: [
          { type: 'file', mediaType: 'image/png', data: 'AAA=' },
          { type: 'file', mediaType: 'audio/wav', data: 'BBB=' },
        ],
      },
      {
        role: 'user',
        content: [{ type: 'file', mediaType: 'image/jpeg', data: 'CCC=' }],
      },
    ])
    expect(out.attachments.map((a) => a.mediaType)).toEqual([
      'image/png',
      'audio/wav',
      'image/jpeg',
    ])
  })
})

describe('mode selection', () => {
  const history: LanguageModelV2Prompt = [
    { role: 'system', content: 'sys' },
    { role: 'user', content: [{ type: 'text', text: 'first' }] },
    { role: 'assistant', content: [{ type: 'text', text: 'reply' }] },
    { role: 'user', content: [{ type: 'text', text: 'second' }] },
  ]

  test('fresh sends the full history', () => {
    const out = call(history, { mode: 'fresh' })
    expect(out.text).toBe(
      'System: sys\nUser: first\nAssistant: reply\nUser: second',
    )
  })

  test('continuation sends only the latest user message', () => {
    const out = call(history, { mode: 'continuation' })
    expect(out.text).toBe('User: second')
  })

  test('continuation with no user messages returns empty text', () => {
    const out = call([{ role: 'system', content: 'sys' }], {
      mode: 'continuation',
    })
    expect(out.text).toBe('')
    expect(out.attachments).toEqual([])
  })

  test('empty prompt returns empty output', () => {
    const out = call([])
    expect(out.text).toBe('')
    expect(out.attachments).toEqual([])
  })
})

describe('JSON response format', () => {
  test('text response format does not prepend instruction', () => {
    const out = call(
      [{ role: 'user', content: [{ type: 'text', text: 'hi' }] }],
      { responseFormat: { type: 'text' } },
    )
    expect(out.text).toBe('User: hi')
  })

  test('json response format prepends a structured-output instruction', () => {
    const out = call(
      [{ role: 'user', content: [{ type: 'text', text: 'hi' }] }],
      { responseFormat: { type: 'json' } },
    )
    expect(out.text).toContain('[Structured Output Instruction]')
    expect(out.text).toContain(
      'You MUST respond with a single valid JSON value.',
    )
    expect(out.text).toContain('[End Structured Output Instruction]')
    expect(out.text.endsWith('User: hi')).toBe(true)
  })

  test('json response format includes name and description when provided', () => {
    const out = call(
      [{ role: 'user', content: [{ type: 'text', text: 'hi' }] }],
      {
        responseFormat: {
          type: 'json',
          name: 'Person',
          description: 'a single person record',
        },
      },
    )
    expect(out.text).toContain('Output name: Person')
    expect(out.text).toContain('Output description: a single person record')
  })

  test('json response format embeds the schema when provided', () => {
    const schema = {
      type: 'object' as const,
      properties: { name: { type: 'string' as const } },
      required: ['name'],
    }
    const out = call(
      [{ role: 'user', content: [{ type: 'text', text: 'hi' }] }],
      { responseFormat: { type: 'json', schema } },
    )
    expect(out.text).toContain(
      'The JSON value MUST conform to this JSON Schema:',
    )
    expect(out.text).toContain('"name"')
  })

  test('json instruction lands on its own line before the prompt', () => {
    const out = call(
      [{ role: 'user', content: [{ type: 'text', text: 'hi' }] }],
      { responseFormat: { type: 'json' } },
    )
    const lines = out.text.split('\n')
    expect(lines[0]).toBe('[Structured Output Instruction]')
    expect(lines.at(-1)).toBe('User: hi')
  })
})
