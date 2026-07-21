import { describe, expect, test } from 'bun:test'
import type {
  LanguageModelV2CallOptions,
  LanguageModelV2StreamPart,
} from '@ai-sdk/provider'
import { AcpxError } from '../../src/errors.ts'
import { createAcpxProvider } from '../../src/index.ts'
import { acpEvent, acpResult } from '../helpers/acp-event-builders.ts'
import { MockAcpRuntime } from '../helpers/mock-acp-runtime.ts'
import { convertReadableStreamToArray } from '../helpers/streams.ts'

function userPrompt(text: string): LanguageModelV2CallOptions['prompt'] {
  return [{ role: 'user', content: [{ type: 'text', text }] }]
}

const baseCall: Pick<LanguageModelV2CallOptions, 'prompt'> = {
  prompt: userPrompt('hello'),
}

async function streamParts(
  runtime: MockAcpRuntime,
  call: LanguageModelV2CallOptions = baseCall as LanguageModelV2CallOptions,
): Promise<LanguageModelV2StreamPart[]> {
  const provider = createAcpxProvider({ agent: 'claude', runtime })
  const { stream } = await provider.languageModel().doStream(call)
  return convertReadableStreamToArray(stream)
}

describe('doStream — basic shapes', () => {
  test('empty turn yields just a finish part', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [{ events: [], result: acpResult.completed('end_turn') }],
    })
    const parts = await streamParts(runtime)
    expect(parts).toHaveLength(1)
    expect(parts[0]).toMatchObject({ type: 'finish', finishReason: 'stop' })
  })

  test('single text turn produces start / delta / end / finish', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('hello')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const parts = await streamParts(runtime)
    const types = parts.map((p) => p.type)
    expect(types).toEqual(['text-start', 'text-delta', 'text-end', 'finish'])
  })

  test('multi-chunk text turn opens one block and emits many deltas', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('a'), acpEvent.text('b'), acpEvent.text('c')],
          result: acpResult.completed(),
        },
      ],
    })
    const parts = await streamParts(runtime)
    const starts = parts.filter((p) => p.type === 'text-start')
    const ends = parts.filter((p) => p.type === 'text-end')
    const deltas = parts.filter((p) => p.type === 'text-delta')
    expect(starts).toHaveLength(1)
    expect(ends).toHaveLength(1)
    expect(deltas.map((p) => (p as { delta: string }).delta).join('')).toBe(
      'abc',
    )
  })

  test('thought + text interleaved closes/opens correctly', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.thought('think'), acpEvent.text('say')],
          result: acpResult.completed(),
        },
      ],
    })
    const parts = await streamParts(runtime)
    expect(parts.map((p) => p.type)).toEqual([
      'reasoning-start',
      'reasoning-delta',
      'reasoning-end',
      'text-start',
      'text-delta',
      'text-end',
      'finish',
    ])
  })
})

describe('doStream — tool calls', () => {
  test('completed tool emits input deltas + tool-call + tool-result', async () => {
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
    const parts = await streamParts(runtime)
    const types = parts.map((p) => p.type)
    expect(types).toEqual([
      'tool-input-start',
      'tool-input-delta',
      'tool-input-end',
      'tool-call',
      'tool-result',
      'finish',
    ])
    expect(parts.at(-1)).toMatchObject({ finishReason: 'tool-calls' })
  })

  test('failed tool sets isError on tool-result', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [
            acpEvent.toolCall({
              toolCallId: 'c1',
              title: 'greet',
              text: 'oops',
              status: 'failed',
            }),
          ],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const parts = await streamParts(runtime)
    const result = parts.find((p) => p.type === 'tool-result')
    expect(result).toMatchObject({ isError: true })
  })

  test('tool call followed by text closes tool block then reopens text', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [
            acpEvent.toolCall({
              toolCallId: 'c1',
              title: 'greet',
              text: '{}',
              status: 'completed',
            }),
            acpEvent.text('done'),
          ],
          result: acpResult.completed(),
        },
      ],
    })
    const parts = await streamParts(runtime)
    const types = parts.map((p) => p.type)
    expect(types).toEqual([
      'tool-input-start',
      'tool-input-delta',
      'tool-input-end',
      'tool-call',
      'tool-result',
      'text-start',
      'text-delta',
      'text-end',
      'finish',
    ])
  })
})

describe('doStream — usage and finish', () => {
  test('usage_update events surface on the finish part', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('hi'), acpEvent.usage(123, 4096)],
          result: acpResult.completed(),
        },
      ],
    })
    const parts = await streamParts(runtime)
    const finish = parts.find((p) => p.type === 'finish')
    expect(finish).toMatchObject({
      type: 'finish',
      usage: {
        inputTokens: undefined,
        outputTokens: undefined,
        totalTokens: 123,
        cachedInputTokens: 4096,
      },
    })
  })

  test('failed turn result yields an error part before the finish part', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [],
          result: acpResult.failed({ message: 'boom', code: 'rate' }),
        },
      ],
    })
    const parts = await streamParts(runtime)

    // The new leading error part — the whole point of issue #32: a
    // consumer iterating fullStream now gets diagnostic data instead
    // of a silent finishReason: 'error'.
    const errorIdx = parts.findIndex((p) => p.type === 'error')
    expect(errorIdx).toBeGreaterThanOrEqual(0)
    const errorPart = parts[errorIdx] as Extract<
      LanguageModelV2StreamPart,
      { type: 'error' }
    >
    expect(errorPart.error).toBeInstanceOf(AcpxError)
    expect((errorPart.error as AcpxError).message).toBe('boom')
    expect((errorPart.error as AcpxError).code).toBe('rate')

    // Finish still comes last with the existing shape — providerMetadata
    // kept verbatim for back-compat with consumers that already read it.
    const finish = parts.at(-1) as Extract<
      LanguageModelV2StreamPart,
      { type: 'finish' }
    >
    expect(finish.type).toBe('finish')
    expect(finish.finishReason).toBe('error')
    expect(finish.providerMetadata?.acpx).toEqual({
      errorCode: 'rate',
      errorMessage: 'boom',
    })

    // Order invariant: error precedes finish.
    expect(errorIdx).toBeLessThan(parts.length - 1)
  })
})

describe('doStream — plan events', () => {
  test('plan events surface as reasoning blocks alongside the answer', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [
            acpEvent.plan('1. read 2. edit 3. test'),
            acpEvent.text('done'),
          ],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const parts = await streamParts(runtime)
    expect(parts.map((p) => p.type)).toEqual([
      'reasoning-start',
      'reasoning-delta',
      'reasoning-end',
      'text-start',
      'text-delta',
      'text-end',
      'finish',
    ])
    const planDelta = parts.find((p) => p.type === 'reasoning-delta')
    expect(planDelta).toMatchObject({
      delta: '[Plan] 1. read 2. edit 3. test',
    })
  })
})

describe('doStream — error path', () => {
  test('rejecting turn.result emits an error part then finish', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [{ events: [], resultError: new Error('ouch') }],
    })
    const parts = await streamParts(runtime)
    const errorPart = parts.find((p) => p.type === 'error')
    expect(errorPart).toBeDefined()
    expect((errorPart as { error: Error }).error.message).toBe('ouch')
  })
})

describe('doStream — JSON response format', () => {
  test('responseFormat type:json strips markdown fences from text-deltas', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('```json\n{"a":1}\n```')],
          result: acpResult.completed(),
        },
      ],
    })
    const parts = await streamParts(runtime, {
      prompt: userPrompt('hi'),
      responseFormat: { type: 'json' },
    } as LanguageModelV2CallOptions)

    const deltas = parts
      .filter((p) => p.type === 'text-delta')
      .map((p) => (p as { delta: string }).delta)
    expect(deltas.join('')).toBe('{"a":1}')
  })

  test('responseFormat type:text leaves output unchanged', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('```json\n{"a":1}\n```')],
          result: acpResult.completed(),
        },
      ],
    })
    const parts = await streamParts(runtime, {
      prompt: userPrompt('hi'),
      responseFormat: { type: 'text' },
    } as LanguageModelV2CallOptions)
    const deltas = parts
      .filter((p) => p.type === 'text-delta')
      .map((p) => (p as { delta: string }).delta)
    expect(deltas.join('')).toContain('```json')
  })
})

describe('doStream — request shape', () => {
  test('request.body identifies agent and sessionKey', async () => {
    const runtime = new MockAcpRuntime()
    const provider = createAcpxProvider({
      agent: 'claude',
      cwd: '/tmp/repo',
      runtime,
    })
    const { request } = await provider
      .languageModel()
      .doStream(baseCall as LanguageModelV2CallOptions)
    expect(request.body).toEqual({
      agent: 'claude',
      sessionKey: 'claude::/tmp/repo',
    })
  })
})
