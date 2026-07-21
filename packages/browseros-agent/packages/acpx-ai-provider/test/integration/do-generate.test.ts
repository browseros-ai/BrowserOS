import { describe, expect, test } from 'bun:test'
import type { LanguageModelV2CallOptions } from '@ai-sdk/provider'
import { AcpxError } from '../../src/errors.ts'
import { createAcpxProvider } from '../../src/index.ts'
import { acpEvent, acpResult } from '../helpers/acp-event-builders.ts'
import { MockAcpRuntime } from '../helpers/mock-acp-runtime.ts'

const baseCall: LanguageModelV2CallOptions = {
  prompt: [{ role: 'user', content: [{ type: 'text', text: 'hi' }] }],
}

describe('doGenerate', () => {
  test('accumulates text deltas into a single text content part', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('hel'), acpEvent.text('lo')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    const result = await provider.languageModel().doGenerate(baseCall)

    expect(result.content).toEqual([{ type: 'text', text: 'hello' }])
    expect(result.finishReason).toBe('stop')
  })

  test('accumulates reasoning deltas separately from text', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.thought('plan'), acpEvent.text('done')],
          result: acpResult.completed(),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    const result = await provider.languageModel().doGenerate(baseCall)

    expect(result.content).toEqual([
      { type: 'reasoning', text: 'plan' },
      { type: 'text', text: 'done' },
    ])
  })

  test('preserves tool-call and tool-result content with providerExecuted flag', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [
            acpEvent.toolCall({
              toolCallId: 'c1',
              title: 'greet',
              text: '{"name":"a"}',
              status: 'completed',
            }),
          ],
          result: acpResult.completed('tool_calls'),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    const result = await provider.languageModel().doGenerate(baseCall)

    expect(result.finishReason).toBe('tool-calls')
    expect(result.content).toEqual([
      {
        type: 'tool-call',
        toolCallId: 'c1',
        toolName: 'greet',
        input: '{"name":"a"}',
        providerExecuted: true,
      },
      {
        type: 'tool-result',
        toolCallId: 'c1',
        toolName: 'greet',
        result: '{"name":"a"}',
      },
    ])
  })

  test('surfaces usage on the result', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('hi'), acpEvent.usage(50, 1024)],
          result: acpResult.completed(),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    const result = await provider.languageModel().doGenerate(baseCall)
    expect(result.usage).toMatchObject({
      totalTokens: 50,
      cachedInputTokens: 1024,
    })
  })

  test('throws when an error part is emitted mid-stream', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [{ events: [], resultError: new Error('crash') }],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    await expect(provider.languageModel().doGenerate(baseCall)).rejects.toThrow(
      'crash',
    )
  })

  test('failed result throws an AcpxError carrying the agent error data', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [],
          result: acpResult.failed({ message: 'boom', code: 'rate' }),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })
    const promise = provider.languageModel().doGenerate(baseCall)

    await expect(promise).rejects.toBeInstanceOf(AcpxError)
    await expect(promise).rejects.toMatchObject({
      code: 'rate',
      message: 'boom',
    })
  })
})
