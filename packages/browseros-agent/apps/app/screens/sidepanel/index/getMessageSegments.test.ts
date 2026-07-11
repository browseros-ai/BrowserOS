import { describe, expect, it } from 'bun:test'
import type { UIMessage } from 'ai'
import { getMessageSegments, stripScaffoldTokens } from './getMessageSegments'

describe('stripScaffoldTokens', () => {
  it('strips the malformed delimiter pair observed from LM Studio (missing pipes), leaving the embedded channel-name word', () => {
    expect(stripScaffoldTokens('<|channel>thought\n<channel|>')).toBe('thought')
  })

  it('strips a token-only string down to empty', () => {
    expect(stripScaffoldTokens('<|end|>')).toBe('')
    expect(stripScaffoldTokens('<|channel|>')).toBe('')
  })

  it('strips well-formed Harmony-style tokens', () => {
    expect(
      stripScaffoldTokens('<|channel|>final<|message|>The answer is 42<|end|>'),
    ).toBe('finalThe answer is 42')
  })

  it('leaves real HTML-like text untouched', () => {
    expect(stripScaffoldTokens('Use a <div> tag here')).toBe(
      'Use a <div> tag here',
    )
  })

  it('leaves markdown tables untouched', () => {
    const table = '| a | b |\n| --- | --- |\n| 1 | 2 |'
    expect(stripScaffoldTokens(table)).toBe(table)
  })

  it('collapses excess blank lines left behind after stripping', () => {
    expect(stripScaffoldTokens('before\n<|end|>\n\n\n\nafter')).toBe(
      'before\n\nafter',
    )
  })

  it('trims surrounding whitespace', () => {
    expect(stripScaffoldTokens('  <|channel|>thought  ')).toBe('thought')
  })
})

function buildMessage(parts: UIMessage['parts']): UIMessage {
  return { id: 'msg-1', role: 'assistant', parts }
}

describe('getMessageSegments scaffold-token handling', () => {
  it('drops a text part that is entirely leaked scaffold tokens', () => {
    const message = buildMessage([{ type: 'text', text: '<|end|>' }])
    expect(getMessageSegments(message, false, false)).toEqual([])
  })

  it('keeps a bare channel-name word left behind when no real content followed', () => {
    const message = buildMessage([
      { type: 'text', text: '<|channel>thought\n<channel|>' },
    ])
    expect(getMessageSegments(message, false, false)).toEqual([
      { type: 'text', key: 'msg-1-text-0', text: 'thought' },
    ])
  })

  it('keeps real text content once scaffold tokens are stripped', () => {
    const message = buildMessage([
      { type: 'text', text: '<|channel|>final<|message|>Hello there<|end|>' },
    ])
    const segments = getMessageSegments(message, false, false)
    expect(segments).toEqual([
      { type: 'text', key: 'msg-1-text-0', text: 'finalHello there' },
    ])
  })

  it('does not disrupt a real tool batch surrounding a leaked segment', () => {
    const message = buildMessage([
      {
        type: 'tool-search',
        toolCallId: 'call-1',
        state: 'output-available',
        input: {},
        output: {},
      },
      { type: 'text', text: '<|end|>' },
      {
        type: 'tool-filesystem_read',
        toolCallId: 'call-2',
        state: 'input-available',
        input: {},
        output: undefined,
      },
    ])
    const segments = getMessageSegments(message, true, true)
    expect(segments.map((s) => s.type)).toEqual(['tool-batch', 'tool-batch'])
  })
})
