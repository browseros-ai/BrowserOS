import { describe, expect, test } from 'bun:test'
import { stepCountIs, streamText } from 'ai'
import { createAcpxProvider } from '../../src/index.ts'
import { acpEvent, acpResult } from '../helpers/acp-event-builders.ts'
import { MockAcpRuntime } from '../helpers/mock-acp-runtime.ts'

describe('streamText — prepare() does not consume session freshness', () => {
  test('prepare() then streamText preserves multi-turn messages on first turn', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('ok')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({
      agent: 'claude',
      sessionKey: 'fresh-prepare-1',
      runtime,
    })

    await provider.prepare()

    const result = streamText({
      model: provider.languageModel(),
      messages: [
        { role: 'user', content: 'What is the capital of France?' },
        { role: 'assistant', content: 'The capital of France is Paris.' },
        { role: 'user', content: 'What did you just tell me?' },
      ],
      stopWhen: stepCountIs(1),
    })
    await result.text

    const turnText = runtime.startTurnCalls[0]?.text ?? ''
    expect(turnText).toContain('What is the capital of France?')
    expect(turnText).toContain('The capital of France is Paris.')
    expect(turnText).toContain('What did you just tell me?')
  })

  test('concurrent doStream calls on the same session: exactly one runs in fresh mode', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('a')],
          result: acpResult.completed('end_turn'),
        },
        {
          events: [acpEvent.text('b')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({
      agent: 'claude',
      sessionKey: 'concurrent-1',
      runtime,
    })

    const messages = [
      { role: 'user' as const, content: 'seed-user-1' },
      { role: 'assistant' as const, content: 'seed-assistant-1' },
      { role: 'user' as const, content: 'seed-user-2' },
    ]

    const first = streamText({
      model: provider.languageModel(),
      messages,
      stopWhen: stepCountIs(1),
    })
    const second = streamText({
      model: provider.languageModel(),
      messages,
      stopWhen: stepCountIs(1),
    })
    await Promise.all([first.text, second.text])

    const texts = runtime.startTurnCalls.map((call) => call.text)
    expect(texts).toHaveLength(2)

    const seededCount = texts.filter((t) =>
      t.includes('seed-assistant-1'),
    ).length
    expect(seededCount).toBe(1)
  })

  test('second turn on the same session sends only the latest user message', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('first')],
          result: acpResult.completed('end_turn'),
        },
        {
          events: [acpEvent.text('second')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({
      agent: 'claude',
      sessionKey: 'continuation-1',
      runtime,
    })

    const first = streamText({
      model: provider.languageModel(),
      messages: [{ role: 'user', content: 'hello first turn' }],
      stopWhen: stepCountIs(1),
    })
    await first.text

    const second = streamText({
      model: provider.languageModel(),
      messages: [
        { role: 'user', content: 'hello first turn' },
        { role: 'assistant', content: 'first' },
        { role: 'user', content: 'follow up message' },
      ],
      stopWhen: stepCountIs(1),
    })
    await second.text

    const secondTurnText = runtime.startTurnCalls[1]?.text ?? ''
    expect(secondTurnText).toContain('follow up message')
    expect(secondTurnText).not.toContain('hello first turn')
    expect(secondTurnText).not.toContain('first')
  })
})
