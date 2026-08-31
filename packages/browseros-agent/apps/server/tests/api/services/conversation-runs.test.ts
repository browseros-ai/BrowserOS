/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import type { UIMessageChunk } from 'ai'
import {
  ConversationRunAlreadyActiveError,
  ConversationRuns,
} from '../../../src/api/services/conversation-runs'

const firstChunk: UIMessageChunk = { type: 'text-start', id: 'answer' }
const secondChunk: UIMessageChunk = {
  type: 'text-delta',
  id: 'answer',
  delta: 'hello',
}

describe('ConversationRuns', () => {
  it('publishes one canonical run to streams and panel assignments', async () => {
    const source = controlledSource()
    const runs = new ConversationRuns()
    const panels = runs.subscribePanelAssignments().getReader()
    expect((await panels.read()).value).toEqual({ assignments: [] })

    const started = await runs.start({
      conversationId: 'conversation-1',
      messages: [],
      panelTabIds: [10, 11],
      createStream: () => source.stream,
    })

    expect((await panels.read()).value).toEqual({
      assignments: [
        {
          tabId: 10,
          conversationId: 'conversation-1',
          runId: started.runId,
          status: 'running',
        },
        {
          tabId: 11,
          conversationId: 'conversation-1',
          runId: started.runId,
          status: 'running',
        },
      ],
    })

    const pinnedRun = runs.activeRun('conversation-1')
    expect(pinnedRun?.associateTabs([12])).toBe(true)
    expect(
      (await panels.read()).value?.assignments.map((tab) => tab.tabId),
    ).toEqual([10, 11, 12])

    const streamed = collect(runs.subscribe('conversation-1'))
    source.write(firstChunk)
    source.close()
    expect(await streamed).toEqual([firstChunk])
    expect((await panels.read()).value?.assignments).toEqual([
      expect.objectContaining({ tabId: 10, status: 'completed' }),
      expect.objectContaining({ tabId: 11, status: 'completed' }),
      expect.objectContaining({ tabId: 12, status: 'completed' }),
    ])

    // A request pins the record that authorized it. Late effects from that
    // record must not attach a tab to a later turn.
    const next = controlledSource()
    await runs.start({
      conversationId: 'conversation-1',
      messages: [],
      panelTabIds: [10],
      createStream: () => next.stream,
    })
    expect(pinnedRun?.associateTabs([13])).toBe(false)
    await runs.stop('conversation-1')
    await panels.cancel()
  })

  it('periodically repeats current panel assignments for client healing', async () => {
    const source = controlledSource()
    const runs = new ConversationRuns({ panelAssignmentsHeartbeatMs: 20 })
    const panels = runs.subscribePanelAssignments().getReader()
    expect((await panels.read()).value).toEqual({ assignments: [] })

    const started = await runs.start({
      conversationId: 'conversation-heartbeat',
      messages: [],
      panelTabIds: [14],
      createStream: () => source.stream,
    })
    const changed = (await panels.read()).value
    const heartbeat = (await panels.read()).value

    expect(changed).toEqual({
      assignments: [
        {
          tabId: 14,
          conversationId: 'conversation-heartbeat',
          runId: started.runId,
          status: 'running',
        },
      ],
    })
    expect(heartbeat).toEqual(changed)

    await panels.cancel()
    await runs.stop('conversation-heartbeat')
  })

  it('keeps replay, multicast, explicit cancellation, and preparation races server-owned', async () => {
    let provideStream!: (stream: ReadableStream<UIMessageChunk>) => void
    const prepared = new Promise<ReadableStream<UIMessageChunk>>((resolve) => {
      provideStream = resolve
    })
    const runs = new ConversationRuns()
    const starting = runs.start({
      conversationId: 'conversation-2',
      messages: [],
      createStream: () => prepared,
    })

    expect(await runs.stop('conversation-2')).toBe(true)
    await expect(
      runs.start({
        conversationId: 'conversation-2',
        messages: [],
        createStream: () => new ReadableStream<UIMessageChunk>(),
      }),
    ).rejects.toBeInstanceOf(ConversationRunAlreadyActiveError)

    provideStream(new ReadableStream<UIMessageChunk>())
    await starting
    expect(runs.getSnapshot('conversation-2')?.status).toBe('aborted')

    const source = controlledSource()
    await runs.start({
      conversationId: 'conversation-2',
      messages: [],
      createStream: () => source.stream,
    })
    const first = collect(runs.subscribe('conversation-2'))
    source.write(firstChunk)
    await eventually(() =>
      expect(runs.getSnapshot('conversation-2')?.chunkCount).toBe(1),
    )
    const late = collect(runs.subscribe('conversation-2'))
    source.write(secondChunk)
    source.close()

    expect(await first).toEqual([firstChunk, secondChunk])
    expect(await late).toEqual([firstChunk, secondChunk])
  })

  it('does not let a subscriber disconnect cancel execution', async () => {
    const source = controlledSource()
    const runs = new ConversationRuns()
    await runs.start({
      conversationId: 'conversation-3',
      messages: [],
      createStream: () => source.stream,
    })

    const abandoned = runs.subscribe('conversation-3').getReader()
    const remaining = collect(runs.subscribe('conversation-3'))
    await abandoned.cancel('panel closed')
    source.write(secondChunk)
    source.close()

    expect(source.cancelReasons).toEqual([])
    expect(await remaining).toEqual([secondChunk])
    expect(runs.getSnapshot('conversation-3')?.status).toBe('completed')
  })

  it('marks an AI SDK error chunk as a failed run and panel assignment', async () => {
    const source = controlledSource()
    const runs = new ConversationRuns()
    const panels = runs.subscribePanelAssignments().getReader()
    await panels.read()
    await runs.start({
      conversationId: 'conversation-error',
      messages: [],
      panelTabIds: [7],
      createStream: () => source.stream,
    })
    await panels.read()

    source.write({ type: 'error', errorText: 'provider unavailable' })
    source.close()

    expect((await panels.read()).value?.assignments[0]?.status).toBe('failed')
    expect(runs.getSnapshot('conversation-error')?.status).toBe('failed')
    await panels.cancel()
  })

  it('waits for preparation cleanup before deletion and blocks replacement', async () => {
    let provideStream!: (stream: ReadableStream<UIMessageChunk>) => void
    const prepared = new Promise<ReadableStream<UIMessageChunk>>((resolve) => {
      provideStream = resolve
    })
    const runs = new ConversationRuns()
    const starting = runs.start({
      conversationId: 'conversation-deleting',
      messages: [],
      createStream: () => prepared,
    })
    let settled = false
    const deleting = runs.delete('conversation-deleting').then((value) => {
      settled = true
      return value
    })
    await Promise.resolve()

    expect(settled).toBe(false)
    await expect(
      runs.start({
        conversationId: 'conversation-deleting',
        messages: [],
        createStream: () => new ReadableStream<UIMessageChunk>(),
      }),
    ).rejects.toBeInstanceOf(ConversationRunAlreadyActiveError)

    provideStream(new ReadableStream<UIMessageChunk>())
    await starting
    expect(await deleting).toBe(true)
    expect(runs.getSnapshot('conversation-deleting')).toBeUndefined()
  })

  it('wakes retained panels without reclaiming tabs owned by another conversation', async () => {
    const runs = new ConversationRuns()
    const first = controlledSource()
    await runs.start({
      conversationId: 'older',
      messages: [],
      panelTabIds: [20],
      createStream: () => first.stream,
    })
    runs.activeRun('older')?.associateTabs([21])
    first.close()
    await eventually(() =>
      expect(runs.getSnapshot('older')?.status).toBe('completed'),
    )

    const newer = controlledSource()
    await runs.start({
      conversationId: 'newer',
      messages: [],
      panelTabIds: [21],
      createStream: () => newer.stream,
    })
    const second = controlledSource()
    await runs.start({
      conversationId: 'older',
      messages: [],
      panelTabIds: [20],
      createStream: () => second.stream,
    })

    const assignments = await currentPanelAssignments(runs)
    expect(assignments.assignments).toEqual([
      expect.objectContaining({ tabId: 20, conversationId: 'older' }),
      expect.objectContaining({ tabId: 21, conversationId: 'newer' }),
    ])
    await runs.stop('newer')
    await runs.stop('older')
  })

  it('keeps scheduled runs out of panel state', async () => {
    const source = controlledSource()
    const runs = new ConversationRuns()
    await runs.start({
      conversationId: 'scheduled',
      messages: [],
      panelTabIds: [30],
      panelsVisible: false,
      createStream: () => source.stream,
    })

    expect(runs.activeRun('scheduled')?.associateTabs([31])).toBe(false)
    expect(await currentPanelAssignments(runs)).toEqual({ assignments: [] })
    await runs.stop('scheduled')
  })

  it('holds panel hydration until preparation publishes canonical messages', async () => {
    let releasePreparation!: () => void
    const preparation = new Promise<void>((resolve) => {
      releasePreparation = resolve
    })
    const runs = new ConversationRuns()
    const starting = runs.start({
      conversationId: 'hydrating',
      messages: [],
      panelTabIds: [32],
      createStream: async (_signal, _runId, updateMessages) => {
        await preparation
        updateMessages([{ id: 'canonical-user', role: 'user', parts: [] }])
        return new ReadableStream<UIMessageChunk>()
      },
    })
    let settled = false
    const hydrating = runs.getPreparedSnapshot('hydrating').then((snapshot) => {
      settled = true
      return snapshot
    })
    await Promise.resolve()
    expect(settled).toBe(false)

    releasePreparation()
    await starting
    expect((await hydrating)?.messages[0]?.id).toBe('canonical-user')
    await runs.stop('hydrating')
  })

  it('rejects stale message updates from a replaced run', async () => {
    const first = controlledSource()
    const runs = new ConversationRuns()
    const firstRun = await runs.start({
      conversationId: 'conversation-messages',
      messages: [],
      createStream: () => first.stream,
    })
    first.close()
    await eventually(() =>
      expect(runs.getSnapshot('conversation-messages')?.status).toBe(
        'completed',
      ),
    )
    const second = controlledSource()
    const secondRun = await runs.start({
      conversationId: 'conversation-messages',
      messages: [],
      createStream: () => second.stream,
    })

    expect(
      runs.updateMessages('conversation-messages', firstRun.runId, [
        { id: 'stale', role: 'assistant', parts: [] },
      ]),
    ).toBe(false)
    expect(
      runs.updateMessages('conversation-messages', secondRun.runId, [
        { id: 'current', role: 'user', parts: [] },
      ]),
    ).toBe(true)
    expect(runs.getSnapshot('conversation-messages')?.messages[0]?.id).toBe(
      'current',
    )
    await runs.stop('conversation-messages')
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

async function currentPanelAssignments(runs: ConversationRuns) {
  const reader = runs.subscribePanelAssignments().getReader()
  const assignments = (await reader.read()).value ?? { assignments: [] }
  await reader.cancel()
  return assignments
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
