import { describe, expect, test } from 'bun:test'
import type { LanguageModelV2StreamPart } from '@ai-sdk/provider'
import {
  createJsonCleanupTransform,
  stripMarkdownFences,
} from '../../src/json-output.ts'

async function collectStream(
  parts: LanguageModelV2StreamPart[],
): Promise<LanguageModelV2StreamPart[]> {
  const input = new ReadableStream<LanguageModelV2StreamPart>({
    start(controller) {
      for (const part of parts) controller.enqueue(part)
      controller.close()
    },
  })

  const reader = input.pipeThrough(createJsonCleanupTransform()).getReader()
  const result: LanguageModelV2StreamPart[] = []
  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    result.push(value)
  }
  return result
}

function collectTextDeltas(parts: LanguageModelV2StreamPart[]): string[] {
  return parts.flatMap((part) =>
    part.type === 'text-delta' ? [part.delta] : [],
  )
}

describe('stripMarkdownFences', () => {
  test('strips ```json fences', () => {
    expect(stripMarkdownFences('```json\n{"a":1}\n```')).toBe('{"a":1}')
  })

  test('strips plain ``` fences', () => {
    expect(stripMarkdownFences('```\n{"a":1}\n```')).toBe('{"a":1}')
  })

  test('returns plain JSON as-is (trimmed)', () => {
    expect(stripMarkdownFences('  {"a":1}  ')).toBe('{"a":1}')
  })

  test('handles multiline JSON in fences', () => {
    expect(stripMarkdownFences('```json\n{\n  "a": 1,\n  "b": 2\n}\n```')).toBe(
      '{\n  "a": 1,\n  "b": 2\n}',
    )
  })

  test('does not strip partial fences', () => {
    expect(stripMarkdownFences('```json\n{"a":1}')).toBe('```json\n{"a":1}')
  })

  test('strips ts language tag fences', () => {
    expect(stripMarkdownFences('```ts\nconst a = 1\n```')).toBe('const a = 1')
  })
})

describe('createJsonCleanupTransform', () => {
  test('strips markdown fences from text blocks', async () => {
    const result = await collectStream([
      { type: 'text-start', id: 't1' },
      { type: 'text-delta', id: 't1', delta: '```json\n' },
      { type: 'text-delta', id: 't1', delta: '{"a":1}' },
      { type: 'text-delta', id: 't1', delta: '\n```' },
      { type: 'text-end', id: 't1' },
    ])

    expect(result).toHaveLength(3)
    expect(result[0]).toEqual({ type: 'text-start', id: 't1' })
    expect(result[1]).toEqual({
      type: 'text-delta',
      id: 't1',
      delta: '{"a":1}',
    })
    expect(result[2]).toEqual({ type: 'text-end', id: 't1' })
  })

  test('preserves incremental streaming for larger JSON payloads', async () => {
    const result = await collectStream([
      { type: 'text-start', id: 't1' },
      { type: 'text-delta', id: 't1', delta: '```json\n{"name":"Tok' },
      {
        type: 'text-delta',
        id: 't1',
        delta: 'yo","country":"Japan","population":14',
      },
      { type: 'text-delta', id: 't1', delta: '000000}\n```' },
      { type: 'text-end', id: 't1' },
    ])

    expect(result[0]).toEqual({ type: 'text-start', id: 't1' })
    expect(result.at(-1)).toEqual({ type: 'text-end', id: 't1' })
    const deltas = collectTextDeltas(result)
    expect(deltas.length).toBeGreaterThanOrEqual(2)
    expect(deltas.join('')).toBe(
      '{"name":"Tokyo","country":"Japan","population":14000000}',
    )
  })

  test('handles fence prefix split across chunks', async () => {
    const result = await collectStream([
      { type: 'text-start', id: 't1' },
      { type: 'text-delta', id: 't1', delta: '```j' },
      { type: 'text-delta', id: 't1', delta: 'son\n{"city":"To' },
      { type: 'text-delta', id: 't1', delta: 'kyo","country":"Japan"}\n```' },
      { type: 'text-end', id: 't1' },
    ])

    expect(collectTextDeltas(result).join('')).toBe(
      '{"city":"Tokyo","country":"Japan"}',
    )
  })

  test('passes through clean JSON text unchanged', async () => {
    const result = await collectStream([
      { type: 'text-start', id: 't1' },
      { type: 'text-delta', id: 't1', delta: '{"a":1}' },
      { type: 'text-end', id: 't1' },
    ])

    expect(result).toHaveLength(3)
    expect(result[1]).toEqual({
      type: 'text-delta',
      id: 't1',
      delta: '{"a":1}',
    })
  })

  test('handles fence with no language tag', async () => {
    const result = await collectStream([
      { type: 'text-start', id: 't1' },
      { type: 'text-delta', id: 't1', delta: '```\n' },
      { type: 'text-delta', id: 't1', delta: '{"a":1}' },
      { type: 'text-delta', id: 't1', delta: '\n```' },
      { type: 'text-end', id: 't1' },
    ])

    expect(collectTextDeltas(result).join('')).toBe('{"a":1}')
  })

  test('handles never-closed fence on text-end (graceful close)', async () => {
    const result = await collectStream([
      { type: 'text-start', id: 't1' },
      { type: 'text-delta', id: 't1', delta: '```json\n{"a":1}' },
      { type: 'text-end', id: 't1' },
    ])

    expect(result.at(-1)?.type).toBe('text-end')
    expect(collectTextDeltas(result).join('')).toBe('{"a":1}')
  })

  test('passes through non-text parts unchanged and in order', async () => {
    const result = await collectStream([
      { type: 'stream-start', warnings: [] },
      { type: 'text-start', id: 't1' },
      { type: 'text-delta', id: 't1', delta: '{"a":1}' },
      { type: 'text-end', id: 't1' },
      { type: 'error', error: new Error('boom') },
    ])

    expect(result).toHaveLength(5)
    expect(result[0]?.type).toBe('stream-start')
    expect(result.at(-1)?.type).toBe('error')
  })

  test('multiple concurrent text blocks are handled independently', async () => {
    const result = await collectStream([
      { type: 'text-start', id: 'a' },
      { type: 'text-start', id: 'b' },
      { type: 'text-delta', id: 'a', delta: '```json\n' },
      { type: 'text-delta', id: 'b', delta: '{"raw":true}' },
      { type: 'text-delta', id: 'a', delta: '{"x":1}\n```' },
      { type: 'text-end', id: 'b' },
      { type: 'text-end', id: 'a' },
    ])

    const aDeltas = result
      .filter((p) => p.type === 'text-delta' && p.id === 'a')
      .flatMap((p) => (p.type === 'text-delta' ? [p.delta] : []))
    const bDeltas = result
      .filter((p) => p.type === 'text-delta' && p.id === 'b')
      .flatMap((p) => (p.type === 'text-delta' ? [p.delta] : []))

    expect(aDeltas.join('')).toBe('{"x":1}')
    expect(bDeltas.join('')).toBe('{"raw":true}')
  })

  test('text-delta without prior text-start is passed through unchanged', async () => {
    const result = await collectStream([
      { type: 'text-delta', id: 'orphan', delta: 'abc' },
    ])

    expect(result).toHaveLength(1)
    expect(result[0]).toEqual({
      type: 'text-delta',
      id: 'orphan',
      delta: 'abc',
    })
  })

  test('text-end without prior text-start is passed through unchanged', async () => {
    const result = await collectStream([{ type: 'text-end', id: 'orphan' }])

    expect(result).toHaveLength(1)
    expect(result[0]).toEqual({ type: 'text-end', id: 'orphan' })
  })

  test('flush emits remaining buffered content when stream closes mid-block', async () => {
    const input = new ReadableStream<LanguageModelV2StreamPart>({
      start(controller) {
        controller.enqueue({ type: 'text-start', id: 't1' })
        controller.enqueue({
          type: 'text-delta',
          id: 't1',
          delta: '```json\n{"a":1}\n```',
        })
        controller.close()
      },
    })

    const reader = input.pipeThrough(createJsonCleanupTransform()).getReader()
    const result: LanguageModelV2StreamPart[] = []
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      result.push(value)
    }

    expect(result[0]?.type).toBe('text-start')
    expect(collectTextDeltas(result).join('')).toBe('{"a":1}')
  })
})
