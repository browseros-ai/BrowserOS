/** The oracle must reject real wire drift while tolerating only documented volatility. */
import { describe, expect, test } from 'bun:test'
import { scenarios, startFakeProvider } from './fake-provider'
import {
  diffBytes,
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
    expect(() => parseSse('data: { "type":"start"}\n\n' + done)).toThrow()
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
})
