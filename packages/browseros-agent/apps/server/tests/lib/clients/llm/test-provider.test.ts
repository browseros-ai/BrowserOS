/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import { testProviderConnection } from '../../../../src/lib/clients/llm/test-provider'

interface StreamTextArgs {
  onError?: (event: { error: unknown }) => void
}
type StreamFactory = (args: StreamTextArgs) => {
  textStream: AsyncIterable<string>
}
type StreamTextFn = NonNullable<Parameters<typeof testProviderConnection>[2]>

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

const BASE_CONFIG = {
  provider: LLM_PROVIDERS.OPENAI_COMPATIBLE,
  model: 'gpt-4o-mini',
  apiKey: 'sk-test',
  baseUrl: 'http://127.0.0.1:8098/v1',
} as const

function streamTextFrom(factory: StreamFactory): StreamTextFn {
  return ((args: StreamTextArgs) => factory(args)) as unknown as StreamTextFn
}

function streamTextWith(textStream: AsyncIterable<string>): StreamTextFn {
  return streamTextFrom(() => ({ textStream }))
}

describe('testProviderConnection', () => {
  it('returns success: false when the stream throws before yielding anything', async () => {
    const result = await testProviderConnection(
      { ...BASE_CONFIG },
      undefined,
      streamTextWith(streamThatThrows('Failed to fetch (127.0.0.1:8098)')),
    )
    expect(result.success).toBe(false)
    expect(result.message).toContain('Failed to fetch')
    expect(result.message).toContain(`[${BASE_CONFIG.provider}]`)
  })

  it('returns success: false when the stream throws mid-stream after a partial chunk', async () => {
    const result = await testProviderConnection(
      { ...BASE_CONFIG },
      undefined,
      streamTextWith(streamThatYieldsThenThrows('partial', 'connection reset')),
    )
    expect(result.success).toBe(false)
    expect(result.message).toContain('connection reset')
    expect(result.message).toContain(`[${BASE_CONFIG.provider}]`)
  })

  it('returns success: false when the SDK invokes onError instead of throwing (real-world: OpenAI 404 on custom baseUrl)', async () => {
    const result = await testProviderConnection(
      { ...BASE_CONFIG },
      undefined,
      streamTextFrom((args) => {
        queueMicrotask(() => {
          args.onError?.({
            error: new Error(
              'AI_APICallError: Not Found (status 404) at https://api.openai.com/coolbro/responses',
            ),
          })
        })
        return { textStream: streamOfChunks([]) }
      }),
    )
    expect(result.success).toBe(false)
    expect(result.message).toContain('AI_APICallError')
    expect(result.message).toContain('404')
    expect(result.message).toContain(`[${BASE_CONFIG.provider}]`)
  })

  it('returns success: true with a response preview when the stream yields chunks', async () => {
    const result = await testProviderConnection(
      { ...BASE_CONFIG },
      undefined,
      streamTextWith(streamOfChunks(['ok'])),
    )
    expect(result.success).toBe(true)
    expect(result.message).toContain('"ok"')
    expect(result.responseTime).toBeGreaterThanOrEqual(0)
  })

  it('returns success: true with a generic message when the stream yields no chunks and no error', async () => {
    const result = await testProviderConnection(
      { ...BASE_CONFIG },
      undefined,
      streamTextWith(streamOfChunks([])),
    )
    expect(result.success).toBe(true)
    expect(result.message).toContain('Provider responded')
  })

  it('truncates the response preview at 100 chars', async () => {
    const result = await testProviderConnection(
      { ...BASE_CONFIG },
      undefined,
      streamTextWith(streamOfChunks(['x'.repeat(200)])),
    )
    expect(result.success).toBe(true)
    expect(result.message).toContain(`${'x'.repeat(100)}...`)
  })
})
