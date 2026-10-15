/** The oracle must reject real wire drift while tolerating only documented volatility. */
import { describe, expect, test } from 'bun:test'
import { scenarios, startFakeProvider } from './fake-provider'
import {
  diffBytes,
  normalizeBuiltinConversation,
  normalizeHttp,
  normalizeSse,
  parseSse,
  validateChunks,
} from './wire'

const frame = (value: unknown) => `data: ${JSON.stringify(value)}\n\n`
const done = 'data: [DONE]\n\n'

describe('wire oracle', () => {
  test('tests that generated ids normalize consistently across references', () => {
    const stream = (id: string) =>
      frame({ type: 'text-start', id }) +
      frame({ type: 'text-delta', id, delta: 'héllo' }) +
      frame({ type: 'text-end', id }) +
      done
    expect(normalizeSse(stream('a'))).toBe(normalizeSse(stream('b')))
    expect(
      normalizeSse(
        frame({ type: 'text-start', id: 'a' }) +
          frame({ type: 'text-end', id: 'b' }) +
          done,
      ),
    ).not.toBe(
      normalizeSse(
        frame({ type: 'text-start', id: 'a' }) +
          frame({ type: 'text-end', id: 'a' }) +
          done,
      ),
    )
  })
  test('tests that F8 only removes messageId from start', () => {
    expect(
      normalizeSse(frame({ type: 'start', messageId: 'uuid' }) + done),
    ).toBe(normalizeSse(frame({ type: 'start' }) + done))
    expect(
      normalizeSse(frame({ type: 'error', errorText: 'different' }) + done),
    ).not.toBe(
      normalizeSse(frame({ type: 'error', errorText: 'error' }) + done),
    )
  })
  test('tests that F6 is restricted to the exact ACP artifact', () => {
    const artifact = frame({
      type: 'tool-input-error',
      toolCallId: 'x',
      toolName: 'snapshot',
      input: '',
      providerExecuted: true,
      dynamic: true,
      errorText: 'The ACP agent failed to respond.',
    })
    expect(normalizeSse(artifact + done, { acp: true })).toBe(done)
    expect(normalizeSse(artifact + done)).not.toBe(done)
    expect(
      normalizeSse(
        artifact.replace('failed to respond.', 'failed differently.') + done,
        { acp: true },
      ),
    ).not.toBe(done)
  })
  test('tests that framing, key order, null and omitted keys remain observable', () => {
    expect(() => parseSse('data: {}\r\n\r\n')).toThrow()
    expect(() => parseSse('data: {}\n\n')).toThrow()
    expect(() => parseSse(`data: { "type":"start"}\n\n${done}`)).toThrow()
    expect(
      diffBytes(
        frame({ type: 'text-start', id: 'x' }),
        frame({ id: 'x', type: 'text-start' }),
      ),
    ).toContain('byte')
    expect(diffBytes('{"a":null}', '{}')).not.toBeNull()
  })
  test('tests that HTTP normalization retains error bodies and array order', () => {
    expect(
      normalizeHttp({ createdAt: 123, error: 'one', rows: [1, 2] }),
    ).not.toEqual(normalizeHttp({ createdAt: 456, error: 'two', rows: [2, 1] }))
    expect(normalizeHttp({ createdAt: 123, a: 1 })).toEqual(
      normalizeHttp({ a: 1, createdAt: 456 }),
    )
  })
  test('tests that only the approved built-in assistant message id difference is tolerated', () => {
    const body = (id: string, role = 'assistant') => ({
      conversation: {
        messages: [{ id, role, parts: [{ type: 'text', text: 'Hello' }] }],
      },
    })
    const uuid = '12345678-1234-4234-8234-123456789abc'
    expect(normalizeBuiltinConversation(body(''))).toEqual(
      normalizeBuiltinConversation(body(uuid)),
    )
    expect(normalizeBuiltinConversation(body('', 'user'))).not.toEqual(
      normalizeBuiltinConversation(body(uuid, 'user')),
    )
    expect(normalizeBuiltinConversation(body('unexpected-id'))).not.toEqual(
      normalizeBuiltinConversation(body('')),
    )
  })
  test('tests that client-supplied timestamps cannot hide zero or seconds regressions', () => {
    const fixed = new Set([1_700_000_000_000, 1_700_000_000_100])
    const expected = {
      startedAt: 1_700_000_000_000,
      completedAt: 1_700_000_000_100,
    }
    expect(normalizeHttp(expected, undefined, undefined, fixed)).not.toEqual(
      normalizeHttp(
        { startedAt: 0, completedAt: 1_700_000_000 },
        undefined,
        undefined,
        fixed,
      ),
    )
    expect(normalizeHttp(expected, undefined, undefined, fixed)).toEqual(
      expected,
    )
  })
  test('tests that the app AI SDK rejects missing chunk fields', async () => {
    await expect(
      validateChunks(frame({ type: 'text-delta', id: 'x' }) + done),
    ).rejects.toThrow()
    await validateChunks(
      frame({ type: 'start' }) +
        frame({ type: 'finish', finishReason: 'stop' }) +
        done,
    )
  })
})

describe('fake upstream', () => {
  test('tests that every scenario speaks each provider dialect', async () => {
    const fake = startFakeProvider({ delayMs: 0 })
    try {
      for (const path of [
        '/v1/chat/completions',
        '/v1/responses',
        '/v1/messages',
      ]) {
        for (const scenario of scenarios) {
          const response = await fetch(`${fake.url}${path}`, {
            method: 'POST',
            headers: {
              'Content-Type': 'application/json',
              'X-Compat-Session': `${path}-${scenario}`,
            },
            body: JSON.stringify({
              model: scenario,
              messages: [],
              input: [],
              stream: true,
            }),
          })
          if (scenario === 'retry-429') {
            expect(response.status).toBe(429)
            const retry = await fetch(`${fake.url}${path}`, {
              method: 'POST',
              headers: {
                'Content-Type': 'application/json',
                'X-Compat-Session': `${path}-${scenario}`,
              },
              body: JSON.stringify({
                model: scenario,
                messages: [],
                input: [],
                stream: true,
              }),
            })
            expect(retry.status).toBe(200)
            await retry.text()
          } else {
            expect(response.status).toBe(200)
            const text = await response.text()
            expect(text.length).toBeGreaterThan(20)
            if (scenario === 'parallel-tools') expect(text).toContain('call_2')
            if (scenario === 'unknown-tool')
              expect(text).toContain('compat_unknown_tool')
            if (scenario === 'mid-stream-error')
              expect(text).toContain('scripted stream failure')
          }
        }
      }
    } finally {
      await fake.stop()
    }
  })
  test('tests that tool follow-up terminates and retry state is isolated', async () => {
    const fake = startFakeProvider({ delayMs: 0 })
    try {
      const response = await fetch(`${fake.url}/v1/chat/completions`, {
        method: 'POST',
        body: JSON.stringify({
          model: 'one-tool',
          messages: [{ role: 'tool', tool_call_id: 'call_1', content: 'done' }],
          stream: true,
        }),
      })
      const text = await response.text()
      expect(text).toContain('Done.')
      expect(text).not.toContain('tool_calls')
      const unknown = await fetch(`${fake.url}/v1/chat/completions`, {
        method: 'POST',
        body: JSON.stringify({ model: 'typo' }),
      })
      expect(unknown.status).toBe(400)
    } finally {
      await fake.stop()
    }
  })
  test('tests that the long upstream pauses at four deltas even when a network reader coalesces frames', async () => {
    const fake = startFakeProvider({ delayMs: 0, pauseLongAfterDeltas: 4 })
    const abort = new AbortController()
    try {
      const response = await fetch(`${fake.url}/v1/chat/completions`, {
        method: 'POST',
        body: JSON.stringify({
          model: 'long-stream',
          stream: true,
          messages: [],
        }),
        signal: abort.signal,
      })
      const reader = response.body?.getReader()
      if (!reader) throw new Error('Missing fake body')
      let raw = ''
      const decoder = new TextDecoder()
      while (!raw.includes('token-3 ')) {
        const part = await reader.read()
        raw += decoder.decode(part.value)
      }
      const pending = reader.read()
      const extra = await Promise.race([
        pending,
        Bun.sleep(30).then(() => 'paused'),
      ])
      expect(extra).toBe('paused')
      expect((raw.match(/token-/g) ?? []).length).toBe(4)
      await reader.cancel()
      abort.abort()
      await pending.catch(() => {})
      for (let i = 0; i < 20 && fake.activeStreams; i++) await Bun.sleep(5)
      expect(fake.activeStreams).toBe(0)
    } finally {
      abort.abort()
      await fake.stop()
    }
  })
})
