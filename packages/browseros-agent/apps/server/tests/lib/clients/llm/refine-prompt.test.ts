/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'
import { refinePrompt } from '../../../../src/lib/clients/llm/refine-prompt'

interface StreamTextArgs {
  onError?: (event: { error: unknown }) => void
}
type StreamFactory = (args: StreamTextArgs) => {
  textStream: AsyncIterable<string>
}
type StreamTextFn = NonNullable<Parameters<typeof refinePrompt>[3]>

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

const REQUEST = { prompt: 'draft a task', name: 'Morning brief' }

function streamTextFrom(factory: StreamFactory): StreamTextFn {
  return ((args: StreamTextArgs) => factory(args)) as unknown as StreamTextFn
}

function streamTextWith(textStream: AsyncIterable<string>): StreamTextFn {
  return streamTextFrom(() => ({ textStream }))
}

describe('refinePrompt', () => {
  it('returns success: false when the stream throws before yielding anything', async () => {
    const result = await refinePrompt(
      { ...BASE_CONFIG },
      REQUEST,
      undefined,
      streamTextWith(streamThatThrows('Failed to fetch (127.0.0.1:8098)')),
    )
    expect(result.success).toBe(false)
    expect(result.message).toContain('Failed to fetch')
    expect(result.refined).toBeUndefined()
  })

  it('returns success: false when the stream throws mid-stream after a partial chunk', async () => {
    const result = await refinePrompt(
      { ...BASE_CONFIG },
      REQUEST,
      undefined,
      streamTextWith(
        streamThatYieldsThenThrows('partial refinement', 'connection reset'),
      ),
    )
    expect(result.success).toBe(false)
    expect(result.message).toContain('connection reset')
    expect(result.refined).toBeUndefined()
  })

  it('returns success: false when the SDK invokes onError instead of throwing', async () => {
    const result = await refinePrompt(
      { ...BASE_CONFIG },
      REQUEST,
      undefined,
      streamTextFrom((args) => {
        queueMicrotask(() => {
          args.onError?.({
            error: new Error('AI_APICallError: Unauthorized (status 401)'),
          })
        })
        return { textStream: streamOfChunks([]) }
      }),
    )
    expect(result.success).toBe(false)
    expect(result.message).toContain('AI_APICallError')
    expect(result.message).toContain('401')
    expect(result.refined).toBeUndefined()
  })

  it('returns success: true with the refined prompt when the stream yields chunks', async () => {
    const result = await refinePrompt(
      { ...BASE_CONFIG },
      REQUEST,
      undefined,
      streamTextWith(
        streamOfChunks(['Open ', 'linkedin.com', ' and ', 'read']),
      ),
    )
    expect(result.success).toBe(true)
    expect(result.refined).toBe('Open linkedin.com and read')
    expect(result.message).toBeUndefined()
  })

  it("returns success: false with 'empty response' when the stream yields nothing and no error", async () => {
    const result = await refinePrompt(
      { ...BASE_CONFIG },
      REQUEST,
      undefined,
      streamTextWith(streamOfChunks([])),
    )
    expect(result.success).toBe(false)
    expect(result.message).toBe('Provider returned an empty response')
    expect(result.refined).toBeUndefined()
  })

  it("treats a whitespace-only response as 'empty response' (trim behavior)", async () => {
    const result = await refinePrompt(
      { ...BASE_CONFIG },
      REQUEST,
      undefined,
      streamTextWith(streamOfChunks(['   ', '\n\t', '  '])),
    )
    expect(result.success).toBe(false)
    expect(result.message).toBe('Provider returned an empty response')
  })
})
