/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Regression coverage: the `/test-provider` probe MUST report
 * `success: false` when the underlying provider errors mid-stream.
 * Previously `test-provider.ts` awaited `stream.text`, which the AI
 * SDK resolves to '' on streaming errors (errors are surfaced via
 * the separate `onError` callback), so a bad URL, DNS failure, or
 * 401 was silently reported as "Provider responded". The fix
 * iterates `stream.textStream` so errors throw and land in the catch.
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'

// Per-test stream factory. Receives the streamText args (so tests
// can inspect / trigger the `onError` callback the caller passes)
// and returns the object streamText would normally return.
interface StreamTextArgs {
  onError?: (event: { error: unknown }) => void
}
type StreamFactory = (args: StreamTextArgs) => {
  textStream: AsyncIterable<string>
}
let currentStreamFactory: StreamFactory | null = null

// Spread the real `ai` module so downstream consumers keep every
// unrelated export (e.g. simulateReadableStream, tool types) and
// only `streamText` gets our test double. Mocking any internal
// `src/lib/**` module here would bleed into sibling tests via Bun's
// mock.module cross-file scope (see feedback_scoped_tests).
const realAi = await import('ai')
mock.module('ai', () => ({
  ...realAi,
  streamText: (args: StreamTextArgs) => {
    if (!currentStreamFactory) {
      throw new Error('test stream not primed')
    }
    return currentStreamFactory(args)
  },
}))

const { testProviderConnection } = await import(
  '../../../../src/lib/clients/llm/test-provider'
)

function streamOfChunks(chunks: string[]): AsyncIterable<string> {
  return {
    async *[Symbol.asyncIterator]() {
      for (const chunk of chunks) {
        yield chunk
      }
    },
  }
}

function streamThatThrows(message: string): AsyncIterable<string> {
  return {
    // biome-ignore lint/correctness/useYield: iterator throws before yielding
    async *[Symbol.asyncIterator](): AsyncGenerator<string> {
      throw new Error(message)
    },
  }
}

function streamThatYieldsThenThrows(
  chunk: string,
  message: string,
): AsyncIterable<string> {
  return {
    async *[Symbol.asyncIterator]() {
      yield chunk
      throw new Error(message)
    },
  }
}

beforeEach(() => {
  currentStreamFactory = null
})

const BASE_CONFIG = {
  provider: LLM_PROVIDERS.OPENAI_COMPATIBLE,
  model: 'gpt-4o-mini',
  apiKey: 'sk-test',
  baseUrl: 'http://127.0.0.1:8098/v1',
} as const

// Sugar over the factory shape: primes a stream that ignores the
// streamText args entirely. Suitable for the "throws" and
// "empty chunks" cases where the caller's `onError` doesn't matter.
function primeStream(textStream: AsyncIterable<string>): void {
  currentStreamFactory = () => ({ textStream })
}

describe('testProviderConnection', () => {
  it('returns success: false when the stream throws before yielding anything', async () => {
    primeStream(streamThatThrows('Failed to fetch (127.0.0.1:8098)'))
    const result = await testProviderConnection({ ...BASE_CONFIG })
    expect(result.success).toBe(false)
    expect(result.message).toContain('Failed to fetch')
    // Server-side probe prefixes the message with the provider name
    // so the client can distinguish a provider failure from a
    // client-side network fault.
    expect(result.message).toContain(`[${BASE_CONFIG.provider}]`)
  })

  it('returns success: false when the stream throws mid-stream after a partial chunk', async () => {
    primeStream(streamThatYieldsThenThrows('partial', 'connection reset'))
    const result = await testProviderConnection({ ...BASE_CONFIG })
    expect(result.success).toBe(false)
    expect(result.message).toContain('connection reset')
    expect(result.message).toContain(`[${BASE_CONFIG.provider}]`)
  })

  it('returns success: false when the SDK invokes onError instead of throwing (real-world: OpenAI 404 on custom baseUrl)', async () => {
    // Regression for the scenario Dani reproduced against a real
    // OpenAI baseUrl of https://api.openai.com/coolbro: OpenAI's edge
    // returns 404, the OpenAI SDK's failedResponseHandler surfaces an
    // APICallError, and streamText intercepts it, calls `onError`,
    // and lets the `textStream` end quietly. If the probe doesn't
    // capture `onError`, the loop iterates zero chunks and we
    // falsely report "Provider responded".
    currentStreamFactory = (args) => {
      // Invoke the caller's onError asynchronously so the stream is
      // consumed first, exactly like the SDK does under real errors.
      queueMicrotask(() => {
        args.onError?.({
          error: new Error(
            'AI_APICallError: Not Found (status 404) at https://api.openai.com/coolbro/responses',
          ),
        })
      })
      return { textStream: streamOfChunks([]) }
    }
    const result = await testProviderConnection({ ...BASE_CONFIG })
    expect(result.success).toBe(false)
    expect(result.message).toContain('AI_APICallError')
    expect(result.message).toContain('404')
    expect(result.message).toContain(`[${BASE_CONFIG.provider}]`)
  })

  it('returns success: true with a response preview when the stream yields chunks', async () => {
    primeStream(streamOfChunks(['ok']))
    const result = await testProviderConnection({ ...BASE_CONFIG })
    expect(result.success).toBe(true)
    expect(result.message).toContain('"ok"')
    expect(result.responseTime).toBeGreaterThanOrEqual(0)
  })

  it('returns success: true with a generic message when the stream yields no chunks and no error', async () => {
    // Behavior preservation: a provider that responds with an empty
    // body BUT does NOT invoke onError still counts as a successful
    // connection at this layer.
    primeStream(streamOfChunks([]))
    const result = await testProviderConnection({ ...BASE_CONFIG })
    expect(result.success).toBe(true)
    expect(result.message).toContain('Provider responded')
  })

  it('truncates the response preview at 100 chars', async () => {
    primeStream(streamOfChunks(['x'.repeat(200)]))
    const result = await testProviderConnection({ ...BASE_CONFIG })
    expect(result.success).toBe(true)
    expect(result.message).toContain(`${'x'.repeat(100)}...`)
  })
})
