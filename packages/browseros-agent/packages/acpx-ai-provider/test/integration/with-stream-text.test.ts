import { describe, expect, test } from 'bun:test'
import { stepCountIs, streamText } from 'ai'
import { createAcpxProvider } from '../../src/index.ts'
import { acpEvent, acpResult } from '../helpers/acp-event-builders.ts'
import { MockAcpRuntime } from '../helpers/mock-acp-runtime.ts'

describe('streamText — text-only turn', () => {
  test('result.text resolves to concatenated deltas', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('hel'), acpEvent.text('lo')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    const result = streamText({
      model: provider.languageModel(),
      prompt: 'say hi',
      stopWhen: stepCountIs(1),
    })
    expect(await result.text).toBe('hello')
  })

  test('finishReason resolves to "stop" for end_turn', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('ok')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    const result = streamText({
      model: provider.languageModel(),
      prompt: 'hi',
      stopWhen: stepCountIs(1),
    })
    expect(await result.finishReason).toBe('stop')
  })

  test('usage resolves with cachedInputTokens from the size field', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('hi'), acpEvent.usage(75, 4096)],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    const result = streamText({
      model: provider.languageModel(),
      prompt: 'hi',
      stopWhen: stepCountIs(1),
    })
    const usage = await result.usage
    expect(usage.cachedInputTokens).toBe(4096)
  })

  test('textStream yields the same content', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('a'), acpEvent.text('b'), acpEvent.text('c')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    const result = streamText({
      model: provider.languageModel(),
      prompt: 'hi',
      stopWhen: stepCountIs(1),
    })
    let acc = ''
    for await (const chunk of result.textStream) acc += chunk
    expect(acc).toBe('abc')
  })
})

describe('streamText — tool-call turn', () => {
  test('finishReason is tool-calls when stopReason is tool_calls', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [
            acpEvent.toolCall({
              toolCallId: 'c1',
              title: 'greet',
              text: '{"name":"world"}',
              status: 'completed',
            }),
          ],
          result: acpResult.completed('tool_calls'),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    const result = streamText({
      model: provider.languageModel(),
      prompt: 'greet world',
      stopWhen: stepCountIs(1),
    })
    expect(await result.finishReason).toBe('tool-calls')
  })
})

describe('streamText — failure', () => {
  test('failed turn surfaces as finishReason "error"', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [],
          result: acpResult.failed({ message: 'boom', code: 'rate' }),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    const result = streamText({
      model: provider.languageModel(),
      prompt: 'hi',
      stopWhen: stepCountIs(1),
      onError: () => {},
    })
    expect(await result.finishReason).toBe('error')
  })
})

describe('streamText — fullStream shape', () => {
  test('emits text-start / text-delta / text-end / finish-step / finish', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('hi')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    const result = streamText({
      model: provider.languageModel(),
      prompt: 'hi',
      stopWhen: stepCountIs(1),
    })
    const types: string[] = []
    for await (const part of result.fullStream) types.push(part.type)
    expect(types).toContain('text-start')
    expect(types).toContain('text-delta')
    expect(types).toContain('text-end')
    expect(types).toContain('finish')
  })
})
