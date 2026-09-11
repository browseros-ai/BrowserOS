import { describe, expect, test } from 'bun:test'
import type { AcpRuntimeEvent } from 'acpx/runtime'
import { readUIMessageStream, streamText } from 'ai'
import { createAcpxProvider } from '../../src/index'
import { MockAcpRuntime } from '../helpers/mock-acp-runtime'

type Content = { type: 'text' | 'reasoning'; text: string }

const text = (
  value: string,
  messageId?: string,
  stream: 'output' | 'thought' = 'output',
): AcpRuntimeEvent => ({ type: 'text_delta', text: value, messageId, stream })

const cases: {
  name: string
  events: AcpRuntimeEvent[]
  expected: Content[]
}[] = [
  {
    name: 'same-message chunks preserve tokens and existing newlines',
    events: [text('Hel', 'a'), text('lo\n\nworld', 'a')],
    expected: [{ type: 'text', text: 'Hello\n\nworld' }],
  },
  {
    name: 'different messages become separate text parts',
    events: [text('First.', 'a'), text('Sec', 'b'), text('ond.', 'b')],
    expected: [
      { type: 'text', text: 'First.' },
      { type: 'text', text: 'Second.' },
    ],
  },
  {
    name: 'an identified reply starts after an unidentified notice',
    events: [text('Adapter notice.'), text('The answer.', 'a')],
    expected: [
      { type: 'text', text: 'Adapter notice.' },
      { type: 'text', text: 'The answer.' },
    ],
  },
  {
    name: 'agents without message IDs keep streaming into one part',
    events: [text('Hel'), text('lo')],
    expected: [{ type: 'text', text: 'Hello' }],
  },
  {
    name: 'missing IDs do not erase the current message identity',
    events: [
      text('First', 'a'),
      text(' continued'),
      text('.', 'a'),
      text('Second.', 'b'),
    ],
    expected: [
      { type: 'text', text: 'First continued.' },
      { type: 'text', text: 'Second.' },
    ],
  },
  {
    name: 'empty chunks can identify a new message before its text',
    events: [text('First.', 'a'), text('', 'b'), text('Second.')],
    expected: [
      { type: 'text', text: 'First.' },
      { type: 'text', text: 'Second.' },
    ],
  },
  {
    name: 'reasoning respects message boundaries too',
    events: [
      text('First plan.', 'a', 'thought'),
      text('New plan.', 'b', 'thought'),
    ],
    expected: [
      { type: 'reasoning', text: 'First plan.' },
      { type: 'reasoning', text: 'New plan.' },
    ],
  },
  {
    name: 'stream changes still separate content within one message',
    events: [
      text('Before', 'a'),
      text('Think', 'a', 'thought'),
      text('After', 'a'),
    ],
    expected: [
      { type: 'text', text: 'Before' },
      { type: 'reasoning', text: 'Think' },
      { type: 'text', text: 'After' },
    ],
  },
]

function providerFor(events: AcpRuntimeEvent[]) {
  return createAcpxProvider({
    agent: 'claude',
    runtime: new MockAcpRuntime({
      turnScripts: [
        { events, result: { status: 'completed', stopReason: 'end_turn' } },
      ],
    }),
  })
}

describe('ACP message boundaries', () => {
  // Exercise the AI SDK conversion as well as the provider: these UI parts
  // are what BrowserOS persists and renders as separate Markdown blocks.
  test.each(cases)('UI stream: $name', async ({ events, expected }) => {
    const provider = providerFor(events)
    const result = streamText({ model: provider.languageModel(), prompt: 'hi' })
    let content: Content[] = []
    for await (const message of readUIMessageStream({
      stream: result.toUIMessageStream(),
    })) {
      content = message.parts.flatMap((part) =>
        part.type === 'text' || part.type === 'reasoning'
          ? [{ type: part.type, text: part.text }]
          : [],
      )
    }
    expect(content).toEqual(expected)
    await provider.close()
  })

  test.each(cases)('doGenerate: $name', async ({ events, expected }) => {
    const provider = providerFor(events)
    const result = await provider.languageModel().doGenerate({
      prompt: [{ role: 'user', content: [{ type: 'text', text: 'hi' }] }],
    })
    expect(result.content).toEqual(expected)
    await provider.close()
  })
})
