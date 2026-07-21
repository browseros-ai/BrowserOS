import { describe, expect, test } from 'bun:test'
import { generateText, stepCountIs } from 'ai'
import { AcpxError } from '../../src/errors.ts'
import { createAcpxProvider } from '../../src/index.ts'
import { acpEvent, acpResult } from '../helpers/acp-event-builders.ts'
import { MockAcpRuntime } from '../helpers/mock-acp-runtime.ts'

describe('generateText — text-only', () => {
  test('returns the concatenated text and a stop reason', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('hel'), acpEvent.text('lo')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    const { text, finishReason } = await generateText({
      model: provider.languageModel(),
      prompt: 'say hi',
      stopWhen: stepCountIs(1),
    })
    expect(text).toBe('hello')
    expect(finishReason).toBe('stop')
  })

  test('exposes accumulated cachedInputTokens from a usage_update', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.text('hi'), acpEvent.usage(42, 1024)],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    const { usage } = await generateText({
      model: provider.languageModel(),
      prompt: 'hi',
      stopWhen: stepCountIs(1),
    })
    expect(usage.cachedInputTokens).toBe(1024)
  })

  test('reasoning content is preserved alongside text', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [acpEvent.thought('plan'), acpEvent.text('done')],
          result: acpResult.completed('end_turn'),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    const { text, content } = await generateText({
      model: provider.languageModel(),
      prompt: 'hi',
      stopWhen: stepCountIs(1),
    })
    expect(text).toBe('done')
    expect(content.some((c) => c.type === 'reasoning')).toBe(true)
  })
})

describe('generateText — tool-call', () => {
  test('toolCalls and toolResults are populated for a completed tool', async () => {
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

    const { toolCalls, toolResults, finishReason } = await generateText({
      model: provider.languageModel(),
      prompt: 'greet',
      stopWhen: stepCountIs(1),
    })
    expect(finishReason).toBe('tool-calls')
    expect(toolCalls).toHaveLength(1)
    expect(toolCalls[0]).toMatchObject({ toolName: 'greet', toolCallId: 'c1' })
    expect(toolResults).toHaveLength(1)
  })
})

describe('generateText — failure', () => {
  test('failed turn throws an AcpxError carrying the agent error data', async () => {
    const runtime = new MockAcpRuntime({
      turnScripts: [
        {
          events: [],
          result: acpResult.failed({ message: 'boom', code: 'rate' }),
        },
      ],
    })
    const provider = createAcpxProvider({ agent: 'claude', runtime })

    const promise = generateText({
      model: provider.languageModel(),
      prompt: 'hi',
      stopWhen: stepCountIs(1),
    })
    await expect(promise).rejects.toBeInstanceOf(AcpxError)
    await expect(promise).rejects.toMatchObject({
      code: 'rate',
      message: 'boom',
    })
  })
})
