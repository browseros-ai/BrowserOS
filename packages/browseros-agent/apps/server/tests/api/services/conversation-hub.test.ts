/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import type { UIMessageChunk } from 'ai'
import {
  ConversationHub,
  ConversationRunAlreadyActiveError,
} from '../../../src/api/services/conversation-hub'

const firstChunk: UIMessageChunk = {
  type: 'text-start',
  id: 'answer',
}
const secondChunk: UIMessageChunk = {
  type: 'text-delta',
  id: 'answer',
  delta: 'hello',
}

describe('ConversationHub', () => {
  it('multicasts one run to every subscriber', async () => {
    const source = controlledSource()
    const hub = new ConversationHub()
    const started = await hub.start({
      conversationId: 'conversation-1',
      messages: [],
      createStream: () => source.stream,
    })

    const first = collect(hub.subscribe('conversation-1'))
    const second = collect(hub.subscribe('conversation-1'))
    source.write(firstChunk)
    source.write(secondChunk)
    source.close()

    expect(await first).toEqual([firstChunk, secondChunk])
    expect(await second).toEqual([firstChunk, secondChunk])
    expect(started.runId).toBeString()
  })

  it('replays buffered chunks before live chunks for a late subscriber', async () => {
    const source = controlledSource()
    const hub = new ConversationHub()
    await hub.start({
      conversationId: 'conversation-2',
      messages: [],
      createStream: () => source.stream,
    })

    source.write(firstChunk)
    await eventually(() =>
      expect(hub.getSnapshot('conversation-2')?.chunkCount).toBe(1),
    )
    const late = collect(hub.subscribe('conversation-2'))
    source.write(secondChunk)
    source.close()

    expect(await late).toEqual([firstChunk, secondChunk])
  })

  it('keeps the run alive when one subscriber disconnects', async () => {
    const source = controlledSource()
    const hub = new ConversationHub()
    await hub.start({
      conversationId: 'conversation-3',
      messages: [],
      createStream: () => source.stream,
    })

    const abandoned = hub.subscribe('conversation-3').getReader()
    const remaining = collect(hub.subscribe('conversation-3'))
    await abandoned.cancel('panel closed')
    source.write(secondChunk)
    source.close()

    expect(source.cancelReasons).toEqual([])
    expect(await remaining).toEqual([secondChunk])
    expect(hub.getSnapshot('conversation-3')?.status).toBe('completed')
  })

  it('cancels only through the explicit stop operation', async () => {
    let observedSignal: AbortSignal | undefined
    const source = controlledSource()
    const hub = new ConversationHub()
    await hub.start({
      conversationId: 'conversation-4',
      messages: [],
      createStream: (signal) => {
        observedSignal = signal
        return source.stream
      },
    })

    expect(await hub.stop('conversation-4')).toBe(true)
    expect(observedSignal?.aborted).toBe(true)
    expect(source.cancelReasons).toHaveLength(1)
    expect(hub.getSnapshot('conversation-4')?.status).toBe('aborted')
  })

  it('marks a normal stream carrying an AI SDK error chunk as failed', async () => {
    const source = controlledSource()
    const hub = new ConversationHub()
    await hub.start({
      conversationId: 'conversation-error',
      messages: [],
      createStream: () => source.stream,
    })

    source.write({ type: 'error', errorText: 'provider unavailable' })
    source.close()
    await eventually(() =>
      expect(hub.getSnapshot('conversation-error')?.status).toBe('failed'),
    )
  })

  it('defers final cleanup when stop arrives during stream preparation', async () => {
    let provideStream!: (stream: ReadableStream<UIMessageChunk>) => void
    const prepared = new Promise<ReadableStream<UIMessageChunk>>((resolve) => {
      provideStream = resolve
    })
    const ended: string[] = []
    const hub = new ConversationHub()
    const starting = hub.start({
      conversationId: 'conversation-preparing',
      messages: [],
      createStream: () => prepared,
      onEnd: (status) => ended.push(status),
    })

    expect(await hub.stop('conversation-preparing')).toBe(true)
    expect(ended).toEqual([])

    provideStream(new ReadableStream<UIMessageChunk>())
    await starting

    expect(ended).toEqual(['aborted'])
    expect(hub.getSnapshot('conversation-preparing')?.status).toBe('aborted')
  })

  it('waits for preparation cleanup before deleting a conversation', async () => {
    let provideStream!: (stream: ReadableStream<UIMessageChunk>) => void
    const prepared = new Promise<ReadableStream<UIMessageChunk>>((resolve) => {
      provideStream = resolve
    })
    const hub = new ConversationHub()
    const starting = hub.start({
      conversationId: 'conversation-deleting',
      messages: [],
      createStream: () => prepared,
    })

    let deleteSettled = false
    const deleting = hub.delete('conversation-deleting').then((deleted) => {
      deleteSettled = true
      return deleted
    })
    await Promise.resolve()
    expect(deleteSettled).toBe(false)

    await expect(
      hub.start({
        conversationId: 'conversation-deleting',
        messages: [],
        createStream: () => new ReadableStream<UIMessageChunk>(),
      }),
    ).rejects.toBeInstanceOf(ConversationRunAlreadyActiveError)

    provideStream(new ReadableStream<UIMessageChunk>())
    await starting
    expect(await deleting).toBe(true)
    expect(hub.getSnapshot('conversation-deleting')).toBeUndefined()
  })

  it('rejects a second active run for the same conversation', async () => {
    const source = controlledSource()
    const hub = new ConversationHub()
    await hub.start({
      conversationId: 'conversation-5',
      messages: [],
      createStream: () => source.stream,
    })

    await expect(
      hub.start({
        conversationId: 'conversation-5',
        messages: [],
        createStream: () => new ReadableStream<UIMessageChunk>(),
      }),
    ).rejects.toBeInstanceOf(ConversationRunAlreadyActiveError)
    await hub.stop('conversation-5')
  })

  it('publishes canonical messages without allowing a stale run to overwrite them', async () => {
    const first = controlledSource()
    const hub = new ConversationHub()
    const firstRun = await hub.start({
      conversationId: 'conversation-6',
      messages: [],
      createStream: () => first.stream,
    })
    first.close()
    await eventually(() =>
      expect(hub.getSnapshot('conversation-6')?.status).toBe('completed'),
    )

    const second = controlledSource()
    const secondRun = await hub.start({
      conversationId: 'conversation-6',
      messages: [],
      createStream: () => second.stream,
    })
    expect(
      hub.updateMessages('conversation-6', firstRun.runId, [
        { id: 'stale', role: 'assistant', parts: [] },
      ]),
    ).toBe(false)
    expect(
      hub.updateMessages('conversation-6', secondRun.runId, [
        { id: 'current', role: 'user', parts: [] },
      ]),
    ).toBe(true)
    expect(hub.getSnapshot('conversation-6')?.messages[0]?.id).toBe('current')
    await hub.stop('conversation-6')
  })
})

function controlledSource() {
  let controller!: ReadableStreamDefaultController<UIMessageChunk>
  const cancelReasons: unknown[] = []
  const stream = new ReadableStream<UIMessageChunk>({
    start(nextController) {
      controller = nextController
    },
    cancel(reason) {
      cancelReasons.push(reason)
    },
  })
  return {
    stream,
    cancelReasons,
    write: (chunk: UIMessageChunk) => controller.enqueue(chunk),
    close: () => controller.close(),
  }
}

async function collect(
  stream: ReadableStream<UIMessageChunk>,
): Promise<UIMessageChunk[]> {
  const chunks: UIMessageChunk[] = []
  for await (const chunk of stream) chunks.push(chunk)
  return chunks
}

async function eventually(assertion: () => void): Promise<void> {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    try {
      assertion()
      return
    } catch {
      await Promise.resolve()
    }
  }
  assertion()
}
