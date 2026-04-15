/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it, mock } from 'bun:test'
import {
  buildKlavisToolSet,
  connectKlavisInBackground,
  type KlavisProxyHandle,
} from '../../../../src/api/services/klavis/strata-proxy'

describe('buildKlavisToolSet', () => {
  it('maps MCP content results into model content parts', async () => {
    const handle: KlavisProxyHandle = {
      tools: [
        {
          name: 'gmail_search',
          description: 'Search Gmail',
          inputSchema: { type: 'object' },
        } as never,
      ],
      inputSchemas: new Map([['gmail_search', {} as never]]),
      callTool: mock(async () => ({
        content: [
          { type: 'text', text: 'Found 2 threads' },
          {
            type: 'image',
            data: 'ZmFrZS1pbWFnZQ==',
            mimeType: 'image/png',
          },
        ],
      })),
      close: async () => {},
    }

    const toolSet = buildKlavisToolSet(handle)
    const searchTool = toolSet.gmail_search

    expect(searchTool).toBeDefined()

    const output = await searchTool.execute?.({})
    const modelOutput = await searchTool.toModelOutput?.({
      toolCallId: 'call-1',
      input: {},
      output,
    })

    expect(modelOutput).toEqual({
      type: 'content',
      value: [
        { type: 'text', text: 'Found 2 threads' },
        {
          type: 'image-data',
          data: 'ZmFrZS1pbWFnZQ==',
          mediaType: 'image/png',
        },
      ],
    })
  })

  it('falls back to JSON output for non-content MCP responses', async () => {
    const handle: KlavisProxyHandle = {
      tools: [
        {
          name: 'notion_lookup',
          description: 'Lookup Notion',
          inputSchema: { type: 'object' },
        } as never,
      ],
      inputSchemas: new Map([['notion_lookup', {} as never]]),
      callTool: mock(async () => ({
        toolResult: {
          pageId: 'abc123',
          title: 'Quarterly Plan',
        },
      })),
      close: async () => {},
    }

    const toolSet = buildKlavisToolSet(handle)
    const lookupTool = toolSet.notion_lookup

    expect(lookupTool).toBeDefined()

    const output = await lookupTool.execute?.({})
    const modelOutput = await lookupTool.toModelOutput?.({
      toolCallId: 'call-2',
      input: {},
      output,
    })

    expect(modelOutput).toEqual({
      type: 'json',
      value: {
        toolResult: {
          pageId: 'abc123',
          title: 'Quarterly Plan',
        },
      },
    })
  })

  it('falls back to image/png when the MCP image result omits a mime type', async () => {
    const handle: KlavisProxyHandle = {
      tools: [
        {
          name: 'drive_preview',
          description: 'Preview Drive file',
          inputSchema: { type: 'object' },
        } as never,
      ],
      inputSchemas: new Map([['drive_preview', {} as never]]),
      callTool: mock(async () => ({
        content: [{ type: 'image', data: 'ZmFrZS1pbWFnZQ==' }],
      })),
      close: async () => {},
    }

    const toolSet = buildKlavisToolSet(handle)
    const previewTool = toolSet.drive_preview
    const output = await previewTool.execute?.({})
    const modelOutput = await previewTool.toModelOutput?.({
      toolCallId: 'call-3',
      input: {},
      output,
    })

    expect(modelOutput).toEqual({
      type: 'content',
      value: [
        {
          type: 'image-data',
          data: 'ZmFrZS1pbWFnZQ==',
          mediaType: 'image/png',
        },
      ],
    })
  })
})

describe('connectKlavisInBackground', () => {
  it('retries in the background until a connection succeeds', async () => {
    const handle: KlavisProxyHandle = {
      tools: [],
      inputSchemas: new Map(),
      callTool: mock(async () => ({ content: [] })),
      close: mock(async () => {}),
    }
    const ref = { handle: null as KlavisProxyHandle | null }
    let attempts = 0

    const stop = connectKlavisInBackground(
      ref,
      { klavisClient: {} as never, browserosId: 'browseros-1' },
      {
        retryDelaysMs: [1],
        connect: async () => {
          attempts++
          if (attempts === 1) {
            throw new Error('boom')
          }
          return handle
        },
      },
    )

    await new Promise((resolve) => setTimeout(resolve, 20))

    expect(attempts).toBe(2)
    expect(ref.handle).toBe(handle)
    stop()
  })

  it('closes a late handle when stopped during an in-flight connect', async () => {
    let releaseConnect: (() => void) | undefined
    const connectStarted = new Promise<void>((resolve) => {
      releaseConnect = resolve
    })
    const handle: KlavisProxyHandle = {
      tools: [],
      inputSchemas: new Map(),
      callTool: mock(async () => ({ content: [] })),
      close: mock(async () => {}),
    }
    const ref = { handle: null as KlavisProxyHandle | null }

    const stop = connectKlavisInBackground(
      ref,
      { klavisClient: {} as never, browserosId: 'browseros-2' },
      {
        connect: async () => {
          await connectStarted
          return handle
        },
      },
    )

    stop()
    releaseConnect?.()
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(ref.handle).toBeNull()
    expect(handle.close).toHaveBeenCalledTimes(1)
  })

  it('cancels a scheduled retry when stopped', async () => {
    let resolveFirstAttempt: (() => void) | undefined
    const firstAttemptDone = new Promise<void>((resolve) => {
      resolveFirstAttempt = resolve
    })
    const ref = { handle: null as KlavisProxyHandle | null }
    let attempts = 0

    const stop = connectKlavisInBackground(
      ref,
      { klavisClient: {} as never, browserosId: 'browseros-3' },
      {
        retryDelaysMs: [20],
        connect: async () => {
          attempts++
          if (attempts === 1) {
            resolveFirstAttempt?.()
            throw new Error('boom')
          }
          return {
            tools: [],
            inputSchemas: new Map(),
            callTool: mock(async () => ({ content: [] })),
            close: mock(async () => {}),
          }
        },
      },
    )

    await firstAttemptDone
    stop()
    await new Promise((resolve) => setTimeout(resolve, 40))

    expect(attempts).toBe(1)
    expect(ref.handle).toBeNull()
  })
})
