import { beforeEach, describe, expect, test } from 'bun:test'
import type {
  LanguageModelV2FinishReason,
  LanguageModelV2StreamPart,
  LanguageModelV2Usage,
} from '@ai-sdk/provider'
import type { AcpRuntimeEvent, AcpRuntimeTurnResult } from 'acpx/runtime'
import { EventTranslator } from '../../src/convert-events.ts'
import { AcpxError } from '../../src/errors.ts'

function makeIdGen(prefix = 'id'): () => string {
  let i = 0
  return () => `${prefix}-${++i}`
}

function newTranslator() {
  return new EventTranslator({ generateId: makeIdGen() })
}

function feed(
  translator: EventTranslator,
  events: AcpRuntimeEvent[],
): LanguageModelV2StreamPart[] {
  return events.flatMap((event) => translator.translate(event))
}

const text = (
  delta: string,
  stream: 'output' | 'thought' = 'output',
): AcpRuntimeEvent => ({ type: 'text_delta', text: delta, stream })

const status = (
  fields: Partial<Extract<AcpRuntimeEvent, { type: 'status' }>>,
): AcpRuntimeEvent => ({ type: 'status', text: '', ...fields })

const tool = (
  fields: Partial<Extract<AcpRuntimeEvent, { type: 'tool_call' }>>,
): AcpRuntimeEvent => ({ type: 'tool_call', text: '', ...fields })

describe('text_delta — output stream', () => {
  test('first delta opens a text block then emits the delta', () => {
    const t = newTranslator()
    expect(feed(t, [text('hello')])).toEqual([
      { type: 'text-start', id: 'id-1' },
      { type: 'text-delta', id: 'id-1', delta: 'hello' },
    ])
  })

  test('subsequent deltas reuse the same block id', () => {
    const t = newTranslator()
    const parts = feed(t, [text('hello '), text('world')])
    expect(parts).toEqual([
      { type: 'text-start', id: 'id-1' },
      { type: 'text-delta', id: 'id-1', delta: 'hello ' },
      { type: 'text-delta', id: 'id-1', delta: 'world' },
    ])
  })

  test('empty deltas do not emit text-delta parts', () => {
    const t = newTranslator()
    const parts = feed(t, [text(''), text('hi')])
    const deltaParts = parts.filter((p) => p.type === 'text-delta')
    expect(deltaParts).toHaveLength(1)
  })

  test('default stream is "output" when stream is unset', () => {
    const t = newTranslator()
    const parts = feed(t, [{ type: 'text_delta', text: 'hi' }])
    expect(parts[0]).toEqual({ type: 'text-start', id: 'id-1' })
  })
})

describe('text_delta — thought stream', () => {
  test('first thought delta opens a reasoning block', () => {
    const t = newTranslator()
    expect(feed(t, [text('thinking', 'thought')])).toEqual([
      { type: 'reasoning-start', id: 'id-1' },
      { type: 'reasoning-delta', id: 'id-1', delta: 'thinking' },
    ])
  })

  test('reasoning deltas accumulate under the same block', () => {
    const t = newTranslator()
    const parts = feed(t, [text('foo', 'thought'), text('bar', 'thought')])
    expect(
      parts.filter((p) => p.type === 'reasoning-delta').map((p) => p.delta),
    ).toEqual(['foo', 'bar'])
  })
})

describe('output ↔ thought transitions', () => {
  test('thought → output closes reasoning then opens text', () => {
    const t = newTranslator()
    const parts = feed(t, [text('think', 'thought'), text('say', 'output')])
    const types = parts.map((p) => p.type)
    expect(types).toEqual([
      'reasoning-start',
      'reasoning-delta',
      'reasoning-end',
      'text-start',
      'text-delta',
    ])
  })

  test('output → thought closes text then opens reasoning', () => {
    const t = newTranslator()
    const parts = feed(t, [text('say', 'output'), text('think', 'thought')])
    expect(parts.map((p) => p.type)).toEqual([
      'text-start',
      'text-delta',
      'text-end',
      'reasoning-start',
      'reasoning-delta',
    ])
  })

  test('block ids are distinct across transitions', () => {
    const t = newTranslator()
    const parts = feed(t, [text('a', 'output'), text('b', 'thought')])
    const ids = parts.flatMap((p) => ('id' in p ? [p.id] : []))
    expect(new Set(ids).size).toBe(2)
  })
})

describe('flush()', () => {
  test('closes an open text block', () => {
    const t = newTranslator()
    feed(t, [text('hi')])
    expect(t.flush()).toEqual([{ type: 'text-end', id: 'id-1' }])
  })

  test('closes an open reasoning block', () => {
    const t = newTranslator()
    feed(t, [text('think', 'thought')])
    expect(t.flush()).toEqual([{ type: 'reasoning-end', id: 'id-1' }])
  })

  test('returns nothing when no block is open', () => {
    expect(newTranslator().flush()).toEqual([])
  })

  test('closes any open tool-input block', () => {
    const t = newTranslator()
    feed(t, [tool({ toolCallId: 't1', title: 'greet', text: 'a' })])
    expect(t.flush()).toEqual([{ type: 'tool-input-end', id: 'id-1' }])
  })
})

describe('tool_call — pending and in_progress', () => {
  test('first event opens tool-input-start with the title as toolName', () => {
    const t = newTranslator()
    const parts = feed(t, [
      tool({
        toolCallId: 't1',
        title: 'greet',
        text: '{"x":1}',
        status: 'pending',
      }),
    ])
    expect(parts).toEqual([
      { type: 'tool-input-start', id: 'id-1', toolName: 'greet' },
      { type: 'tool-input-delta', id: 'id-1', delta: '{"x":1}' },
    ])
  })

  test('falls back to "tool" when title is missing', () => {
    const t = newTranslator()
    const parts = feed(t, [tool({ toolCallId: 't1' })])
    const start = parts.find((p) => p.type === 'tool-input-start')
    expect(start).toEqual({
      type: 'tool-input-start',
      id: 'id-1',
      toolName: 'tool',
    })
  })

  test('subsequent updates only emit the new text suffix', () => {
    const t = newTranslator()
    const parts = feed(t, [
      tool({
        toolCallId: 't1',
        title: 'greet',
        text: '{"x":1',
        status: 'pending',
      }),
      tool({
        toolCallId: 't1',
        title: 'greet',
        text: '{"x":1,"y":2}',
        status: 'in_progress',
      }),
    ])
    const deltas = parts
      .filter((p) => p.type === 'tool-input-delta')
      .map((p) => p.delta)
    expect(deltas).toEqual(['{"x":1', ',"y":2}'])
  })

  test('event with no toolCallId is ignored', () => {
    const t = newTranslator()
    const parts = feed(t, [tool({ text: 'orphan' })])
    expect(parts).toEqual([])
  })

  test('tool_call closes any open text block before opening tool-input', () => {
    const t = newTranslator()
    const parts = feed(t, [
      text('hello'),
      tool({ toolCallId: 't1', title: 'greet', text: '{}' }),
    ])
    const types = parts.map((p) => p.type)
    expect(types).toEqual([
      'text-start',
      'text-delta',
      'text-end',
      'tool-input-start',
      'tool-input-delta',
    ])
  })

  test('does not emit a delta when text does not advance', () => {
    const t = newTranslator()
    const parts = feed(t, [
      tool({
        toolCallId: 't1',
        title: 'greet',
        text: 'abc',
        status: 'pending',
      }),
      tool({
        toolCallId: 't1',
        title: 'greet',
        text: 'abc',
        status: 'in_progress',
      }),
    ])
    const deltas = parts
      .filter((p) => p.type === 'tool-input-delta')
      .map((p) => p.delta)
    expect(deltas).toEqual(['abc'])
  })

  test('falls back to emitting full text when prefix invariant breaks', () => {
    const t = newTranslator()
    const parts = feed(t, [
      tool({
        toolCallId: 't1',
        title: 'greet',
        text: 'abc',
        status: 'pending',
      }),
      tool({
        toolCallId: 't1',
        title: 'greet',
        text: 'xyz',
        status: 'in_progress',
      }),
    ])
    const deltas = parts
      .filter((p) => p.type === 'tool-input-delta')
      .map((p) => p.delta)
    expect(deltas).toEqual(['abc', 'xyz'])
  })
})

describe('tool_call — completed and failed', () => {
  test('completed emits tool-input-end + tool-call + tool-result', () => {
    const t = newTranslator()
    const parts = feed(t, [
      tool({
        toolCallId: 't1',
        title: 'greet',
        text: '{"name":"world"}',
        status: 'completed',
      }),
    ])
    const types = parts.map((p) => p.type)
    expect(types).toEqual([
      'tool-input-start',
      'tool-input-delta',
      'tool-input-end',
      'tool-call',
      'tool-result',
    ])

    const call = parts.find((p) => p.type === 'tool-call')
    expect(call).toMatchObject({
      type: 'tool-call',
      toolCallId: 't1',
      toolName: 'greet',
      input: '{"name":"world"}',
      providerExecuted: true,
    })

    const result = parts.find((p) => p.type === 'tool-result')
    expect(result).toMatchObject({
      type: 'tool-result',
      toolCallId: 't1',
      toolName: 'greet',
      result: '{"name":"world"}',
    })
    expect((result as { isError?: boolean }).isError).toBeUndefined()
  })

  test('failed emits the same parts but with isError on tool-result', () => {
    const t = newTranslator()
    const parts = feed(t, [
      tool({
        toolCallId: 't1',
        title: 'greet',
        text: 'oops',
        status: 'failed',
      }),
    ])
    const result = parts.find((p) => p.type === 'tool-result')
    expect(result).toMatchObject({ type: 'tool-result', isError: true })
  })

  test('terminal status without prior pending event still produces full lifecycle', () => {
    const t = newTranslator()
    const parts = feed(t, [
      tool({
        toolCallId: 't1',
        title: 'greet',
        text: 'done',
        status: 'completed',
      }),
    ])
    const types = parts.map((p) => p.type)
    expect(types).toEqual([
      'tool-input-start',
      'tool-input-delta',
      'tool-input-end',
      'tool-call',
      'tool-result',
    ])
  })

  test('after terminal, the tool id is forgotten so a re-use opens a new block', () => {
    const t = newTranslator()
    feed(t, [
      tool({
        toolCallId: 't1',
        title: 'greet',
        text: 'a',
        status: 'completed',
      }),
    ])
    const parts = feed(t, [
      tool({ toolCallId: 't1', title: 'greet', text: 'b', status: 'pending' }),
    ])
    const start = parts.find((p) => p.type === 'tool-input-start')
    expect(start).toBeDefined()
  })
})

describe('status — usage_update', () => {
  test('captures used and size for a later finish() call', () => {
    const t = newTranslator()
    const parts = feed(t, [
      status({ tag: 'usage_update', used: 100, size: 4096 }),
    ])
    expect(parts).toEqual([])

    const finishPart = t.finish({ result: { status: 'completed' } })
    expect(finishPart).toMatchObject({
      type: 'finish',
      finishReason: 'stop',
      usage: {
        inputTokens: undefined,
        outputTokens: undefined,
        totalTokens: 100,
        cachedInputTokens: 4096,
      },
    })
  })

  test('non-usage status events emit nothing and leave usage at default', () => {
    const t = newTranslator()
    const parts = feed(t, [
      status({ tag: 'current_mode_update', text: 'auto' }),
      status({ tag: 'available_commands_update' }),
    ])
    expect(parts).toEqual([])

    const finishPart = t.finish({ result: { status: 'completed' } })
    expect(finishPart).toMatchObject({
      usage: {
        inputTokens: undefined,
        outputTokens: undefined,
        totalTokens: undefined,
      },
    })
  })

  test('multiple usage_update events overwrite earlier values', () => {
    const t = newTranslator()
    feed(t, [
      status({ tag: 'usage_update', used: 100 }),
      status({ tag: 'usage_update', used: 250, size: 8192 }),
    ])
    const finishPart = t.finish({ result: { status: 'completed' } }) as Extract<
      LanguageModelV2StreamPart,
      { type: 'finish' }
    >
    expect(finishPart.usage.totalTokens).toBe(250)
  })
})

describe('status — plan', () => {
  test('emits a self-contained reasoning block with [Plan] prefix', () => {
    const t = newTranslator()
    const parts = feed(t, [
      status({ tag: 'plan', text: '1. read file 2. fix bug 3. test' }),
    ])
    expect(parts).toEqual([
      { type: 'reasoning-start', id: 'id-1' },
      {
        type: 'reasoning-delta',
        id: 'id-1',
        delta: '[Plan] 1. read file 2. fix bug 3. test',
      },
      { type: 'reasoning-end', id: 'id-1' },
    ])
  })

  test('plan event with empty text emits nothing', () => {
    const t = newTranslator()
    const parts = feed(t, [status({ tag: 'plan', text: '' })])
    expect(parts).toEqual([])
  })

  test('plan event with whitespace-only text emits nothing', () => {
    const t = newTranslator()
    const parts = feed(t, [status({ tag: 'plan', text: '   \n\t  ' })])
    expect(parts).toEqual([])
  })

  test('plan text is trimmed before being embedded in the reasoning delta', () => {
    const t = newTranslator()
    const parts = feed(t, [status({ tag: 'plan', text: '  do the thing  ' })])
    const delta = parts.find((p) => p.type === 'reasoning-delta')
    expect(delta).toMatchObject({ delta: '[Plan] do the thing' })
  })

  test('plan during an open thought block does not disturb that block', () => {
    const t = newTranslator()
    const parts = feed(t, [
      text('thinking…', 'thought'),
      status({ tag: 'plan', text: 'I will look up X' }),
      text(' more thinking', 'thought'),
    ])
    const types = parts.map((p) => p.type)
    // the thought block stays open across the plan: reasoning-start
    // for the thought (id-1), thought delta, then a SEPARATE reasoning
    // triplet for the plan (id-2), then more thought-deltas on id-1.
    expect(types).toEqual([
      'reasoning-start',
      'reasoning-delta',
      'reasoning-start',
      'reasoning-delta',
      'reasoning-end',
      'reasoning-delta',
    ])
    const ids = parts.flatMap((p) => ('id' in p ? [p.id] : []))
    expect(new Set(ids).size).toBe(2)
  })

  test('multiple plan events each emit their own block', () => {
    const t = newTranslator()
    const parts = feed(t, [
      status({ tag: 'plan', text: 'first plan' }),
      status({ tag: 'plan', text: 'revised plan' }),
    ])
    const startIds = parts
      .filter((p) => p.type === 'reasoning-start')
      .map((p) => p.id)
    expect(startIds).toHaveLength(2)
    expect(new Set(startIds).size).toBe(2)
    const deltas = parts
      .filter((p) => p.type === 'reasoning-delta')
      .map((p) => p.delta)
    expect(deltas).toEqual(['[Plan] first plan', '[Plan] revised plan'])
  })

  test('plan does not advance currentBlock state, so a follow-up text opens its own text block', () => {
    const t = newTranslator()
    const parts = feed(t, [
      status({ tag: 'plan', text: 'I will…' }),
      text('hello'),
    ])
    expect(parts.map((p) => p.type)).toEqual([
      'reasoning-start',
      'reasoning-delta',
      'reasoning-end',
      'text-start',
      'text-delta',
    ])
  })
})

describe('error events', () => {
  test('produces an error part with an AcpxError instance', () => {
    const t = newTranslator()
    const parts = feed(t, [
      { type: 'error', message: 'boom', code: 'failure', retryable: true },
    ])
    expect(parts).toHaveLength(1)
    expect(parts[0]).toMatchObject({ type: 'error' })
    expect((parts[0] as { error: unknown }).error).toBeInstanceOf(AcpxError)
  })

  test('error event preserves runtime code and retryable flag', () => {
    const t = newTranslator()
    const parts = feed(t, [
      {
        type: 'error',
        message: 'rate limited',
        code: 'rate_limit',
        retryable: true,
      },
    ])
    const error = (parts[0] as { error: AcpxError }).error
    expect(error.code).toBe('rate_limit')
    expect(error.retryable).toBe(true)
  })
})

describe('done events', () => {
  test('emit nothing — terminal handled by finish()', () => {
    const t = newTranslator()
    expect(feed(t, [{ type: 'done', stopReason: 'end_turn' }])).toEqual([])
  })
})

describe('finish() — stop reason mapping', () => {
  let t: EventTranslator
  beforeEach(() => {
    t = newTranslator()
  })

  type Case = {
    label: string
    result: AcpRuntimeTurnResult
    finishReason: LanguageModelV2FinishReason
  }

  const cases: Case[] = [
    {
      label: 'completed + end_turn → stop',
      result: { status: 'completed', stopReason: 'end_turn' },
      finishReason: 'stop',
    },
    {
      label: 'completed + stop_sequence → stop',
      result: { status: 'completed', stopReason: 'stop_sequence' },
      finishReason: 'stop',
    },
    {
      label: 'completed + no stopReason → stop',
      result: { status: 'completed' },
      finishReason: 'stop',
    },
    {
      label: 'completed + max_tokens → length',
      result: { status: 'completed', stopReason: 'max_tokens' },
      finishReason: 'length',
    },
    {
      label: 'completed + tool_calls → tool-calls',
      result: { status: 'completed', stopReason: 'tool_calls' },
      finishReason: 'tool-calls',
    },
    {
      label: 'completed + tool_use → tool-calls',
      result: { status: 'completed', stopReason: 'tool_use' },
      finishReason: 'tool-calls',
    },
    {
      label: 'completed + unknown stopReason → unknown',
      result: { status: 'completed', stopReason: 'made_up' },
      finishReason: 'unknown',
    },
    {
      label: 'cancelled → other',
      result: { status: 'cancelled' },
      finishReason: 'other',
    },
    {
      label: 'failed → error',
      result: { status: 'failed', error: { message: 'boom', code: 'x' } },
      finishReason: 'error',
    },
  ]

  for (const { label, result, finishReason } of cases) {
    test(label, () => {
      const part = t.finish({ result }) as Extract<
        LanguageModelV2StreamPart,
        { type: 'finish' }
      >
      expect(part.finishReason).toBe(finishReason)
    })
  }

  test('failed result attaches errorCode/errorMessage to providerMetadata', () => {
    const part = t.finish({
      result: { status: 'failed', error: { message: 'boom', code: 'rate' } },
    }) as Extract<LanguageModelV2StreamPart, { type: 'finish' }>
    expect(part.providerMetadata).toEqual({
      acpx: { errorCode: 'rate', errorMessage: 'boom' },
    })
  })

  test('explicit usage option overrides accumulated usage', () => {
    feed(t, [status({ tag: 'usage_update', used: 1, size: 2 })])
    const usage: LanguageModelV2Usage = {
      inputTokens: 7,
      outputTokens: 8,
      totalTokens: 15,
    }
    const part = t.finish({
      result: { status: 'completed' },
      usage,
    }) as Extract<LanguageModelV2StreamPart, { type: 'finish' }>
    expect(part.usage).toEqual(usage)
  })
})

describe('EventTranslator — errorPartIfFailed', () => {
  test('returns [] for completed results', () => {
    const t = newTranslator()
    expect(
      t.errorPartIfFailed({ status: 'completed', stopReason: 'end_turn' }),
    ).toEqual([])
  })

  test('returns [] for cancelled results', () => {
    const t = newTranslator()
    expect(t.errorPartIfFailed({ status: 'cancelled' })).toEqual([])
  })

  test('returns one error part with mapped AcpxError for failed results', () => {
    const t = newTranslator()
    const acpError = { message: 'boom', code: 'rate_limit' }
    const parts = t.errorPartIfFailed({ status: 'failed', error: acpError })
    expect(parts).toHaveLength(1)
    const part = parts[0] as Extract<
      LanguageModelV2StreamPart,
      { type: 'error' }
    >
    expect(part.type).toBe('error')
    expect(part.error).toBeInstanceOf(AcpxError)
    const error = part.error as AcpxError
    expect(error.code).toBe('rate_limit')
    expect(error.message).toBe('boom')
    expect(error.cause).toBe(acpError)
  })
})
