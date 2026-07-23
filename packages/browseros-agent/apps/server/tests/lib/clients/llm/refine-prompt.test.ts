/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Regression coverage for `refinePrompt`, mirroring
 * `test-provider.test.ts`. Both call sites had the same
 * `stream.text` bug (SDK errors resolve to '' instead of rejecting),
 * so both need the same "iterate `textStream` and let errors throw"
 * shape locked in against a future re-introduction.
 */

import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { LLM_PROVIDERS } from '@browseros/shared/schemas/llm'

interface StreamTextArgs {
  onError?: (event: { error: unknown }) => void
}
type StreamFactory = (args: StreamTextArgs) => {
  textStream: AsyncIterable<string>
}
let currentStreamFactory: StreamFactory | null = null

// Spread the real `ai` module so downstream consumers keep every
// unrelated export and only `streamText` gets our test double.
// See sibling `test-provider.test.ts` for the same pattern.
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

const { refinePrompt } = await import(
  '../../../../src/lib/clients/llm/refine-prompt'
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

const REQUEST = { prompt: 'draft a task', name: 'Morning brief' }

function primeStream(textStream: AsyncIterable<string>): void {
  currentStreamFactory = () => ({ textStream })
}

describe('refinePrompt', () => {
  it('returns success: false when the stream throws before yielding anything', async () => {
    primeStream(streamThatThrows('Failed to fetch (127.0.0.1:8098)'))
    const result = await refinePrompt({ ...BASE_CONFIG }, REQUEST)
    expect(result.success).toBe(false)
    expect(result.message).toContain('Failed to fetch')
    expect(result.refined).toBeUndefined()
  })

  it('returns success: false when the stream throws mid-stream after a partial chunk', async () => {
    primeStream(
      streamThatYieldsThenThrows('partial refinement', 'connection reset'),
    )
    const result = await refinePrompt({ ...BASE_CONFIG }, REQUEST)
    expect(result.success).toBe(false)
    expect(result.message).toContain('connection reset')
    expect(result.refined).toBeUndefined()
  })

  it('returns success: false when the SDK invokes onError instead of throwing', async () => {
    // Same real-world failure mode as the sibling probe test: the SDK
    // converts a provider APICallError into an `onError` callback and
    // ends the textStream quietly. Without capturing `onError`, the
    // loop iterates zero chunks and we misreport as "empty response".
    currentStreamFactory = (args) => {
      queueMicrotask(() => {
        args.onError?.({
          error: new Error('AI_APICallError: Unauthorized (status 401)'),
        })
      })
      return { textStream: streamOfChunks([]) }
    }
    const result = await refinePrompt({ ...BASE_CONFIG }, REQUEST)
    expect(result.success).toBe(false)
    expect(result.message).toContain('AI_APICallError')
    expect(result.message).toContain('401')
    expect(result.refined).toBeUndefined()
  })

  it('returns success: true with the refined prompt when the stream yields chunks', async () => {
    primeStream(streamOfChunks(['Open ', 'linkedin.com', ' and ', 'read']))
    const result = await refinePrompt({ ...BASE_CONFIG }, REQUEST)
    expect(result.success).toBe(true)
    expect(result.refined).toBe('Open linkedin.com and read')
    expect(result.message).toBeUndefined()
  })

  it("returns success: false with 'empty response' when the stream yields nothing and no error", async () => {
    primeStream(streamOfChunks([]))
    const result = await refinePrompt({ ...BASE_CONFIG }, REQUEST)
    expect(result.success).toBe(false)
    expect(result.message).toBe('Provider returned an empty response')
    expect(result.refined).toBeUndefined()
  })

  it("treats a whitespace-only response as 'empty response' (trim behavior)", async () => {
    // `.trim()` on the joined content means a stream of blank space
    // is indistinguishable from truly-empty and returns the same
    // failure. Guards against a future refactor that skips the trim
    // and reports whitespace as success.
    primeStream(streamOfChunks(['   ', '\n\t', '  ']))
    const result = await refinePrompt({ ...BASE_CONFIG }, REQUEST)
    expect(result.success).toBe(false)
    expect(result.message).toBe('Provider returned an empty response')
  })
})
