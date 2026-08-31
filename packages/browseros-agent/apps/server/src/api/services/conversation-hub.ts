/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { ConversationRunStatus } from '@browseros/shared/schemas/conversation-presence'
import type { UIMessage, UIMessageChunk } from 'ai'

export type { ConversationRunStatus } from '@browseros/shared/schemas/conversation-presence'

export interface ConversationRunSnapshot {
  conversationId: string
  runId: string
  status: ConversationRunStatus
  messages: UIMessage[]
  chunkCount: number
}

export interface StartConversationRunInput {
  conversationId: string
  messages: UIMessage[]
  createStream: (
    signal: AbortSignal,
    runId: string,
    updateMessages: (messages: UIMessage[]) => boolean,
  ) => ReadableStream<UIMessageChunk> | Promise<ReadableStream<UIMessageChunk>>
  onEnd?: (
    status: Exclude<ConversationRunStatus, 'running'>,
    runId: string,
  ) => Promise<void> | void
}

export interface StartedConversationRun {
  runId: string
}

interface ConversationRunRecord {
  conversationId: string
  runId: string
  status: ConversationRunStatus
  messages: UIMessage[]
  chunks: UIMessageChunk[]
  abortController: AbortController
  reader?: ReadableStreamDefaultReader<UIMessageChunk>
  subscribers: Set<ReadableStreamDefaultController<UIMessageChunk>>
  onEnd?: StartConversationRunInput['onEnd']
  sawErrorChunk: boolean
  ended: boolean
  deleting: boolean
  finished: Promise<void>
  resolveFinished: () => void
}

export class ConversationRunAlreadyActiveError extends Error {
  constructor() {
    super('A conversation run is already active')
    this.name = 'ConversationRunAlreadyActiveError'
  }
}

export class ConversationRunNotFoundError extends Error {
  constructor() {
    super('Conversation run not found')
    this.name = 'ConversationRunNotFoundError'
  }
}

/**
 * Owns each agent run independently of any HTTP response and fans its ordered
 * UI chunks out to every attached panel. Subscriber cancellation only detaches
 * that view; `stop` is the sole operation that cancels model execution.
 */
export class ConversationHub {
  private readonly runs = new Map<string, ConversationRunRecord>()

  async start(
    input: StartConversationRunInput,
  ): Promise<StartedConversationRun> {
    const existing = this.runs.get(input.conversationId)
    if (existing?.status === 'running') {
      throw new ConversationRunAlreadyActiveError()
    }

    if (existing?.deleting) throw new ConversationRunAlreadyActiveError()

    let resolveFinished!: () => void
    const finished = new Promise<void>((resolve) => {
      resolveFinished = resolve
    })

    const record: ConversationRunRecord = {
      conversationId: input.conversationId,
      runId: crypto.randomUUID(),
      status: 'running',
      messages: [...input.messages],
      chunks: [],
      abortController: new AbortController(),
      subscribers: new Set(),
      onEnd: input.onEnd,
      sawErrorChunk: false,
      ended: false,
      deleting: false,
      finished,
      resolveFinished,
    }
    // Install the record before awaiting stream construction. This closes the
    // preparation race where two POSTs could both create an agent turn.
    this.runs.set(input.conversationId, record)

    try {
      const stream = await input.createStream(
        record.abortController.signal,
        record.runId,
        (messages) =>
          this.updateMessages(record.conversationId, record.runId, messages),
      )
      record.reader = stream.getReader()
      if (record.abortController.signal.aborted) {
        await record.reader.cancel(record.abortController.signal.reason)
        await this.finish(record, 'aborted')
      } else {
        void this.pump(record)
      }
      return { runId: record.runId }
    } catch (error) {
      await this.finish(
        record,
        record.abortController.signal.aborted ? 'aborted' : 'failed',
      )
      throw error
    }
  }

  /** Replays buffered chunks, then attaches to the same live ordered stream. */
  subscribe(conversationId: string): ReadableStream<UIMessageChunk> {
    const record = this.runs.get(conversationId)
    if (!record) throw new ConversationRunNotFoundError()
    let subscriber: ReadableStreamDefaultController<UIMessageChunk> | undefined

    return new ReadableStream<UIMessageChunk>({
      start: (controller) => {
        subscriber = controller
        for (const chunk of record.chunks) controller.enqueue(chunk)
        if (record.status === 'running') {
          record.subscribers.add(controller)
        } else {
          controller.close()
        }
      },
      // Deliberately do not cancel the source reader here. A browser tab or
      // panel is only a view of the server-owned run, never its lifecycle owner.
      cancel: () => {
        if (subscriber) record.subscribers.delete(subscriber)
      },
    })
  }

  async stop(conversationId: string): Promise<boolean> {
    const record = this.runs.get(conversationId)
    if (record?.status !== 'running') return false

    const reason = new DOMException('Conversation stopped', 'AbortError')
    record.abortController.abort(reason)
    // During provider/MCP preparation there is no source reader yet. Defer
    // finalization until `start` receives (or fails to receive) the stream, so
    // cleanup runs after late-created leases and presence have been registered.
    if (!record.reader) return true

    await record.reader.cancel(reason).catch(() => undefined)
    await this.finish(record, 'aborted')
    return true
  }

  async delete(conversationId: string): Promise<boolean> {
    const record = this.runs.get(conversationId)
    if (!record) return false
    // Block a replacement turn while a late stream factory unwinds. Callers
    // such as ChatService can safely delete sessions and revoke leases after
    // this promise resolves without a ghost session appearing behind them.
    record.deleting = true
    if (record.status === 'running') await this.stop(conversationId)
    await record.finished
    return this.runs.get(conversationId) === record
      ? this.runs.delete(conversationId)
      : false
  }

  getSnapshot(conversationId: string): ConversationRunSnapshot | undefined {
    const record = this.runs.get(conversationId)
    if (!record) return undefined
    return {
      conversationId: record.conversationId,
      runId: record.runId,
      status: record.status,
      messages: [...record.messages],
      chunkCount: record.chunks.length,
    }
  }

  updateMessages(
    conversationId: string,
    runId: string,
    messages: UIMessage[],
  ): boolean {
    const record = this.runs.get(conversationId)
    if (!record || record.runId !== runId) return false
    record.messages = [...messages]
    return true
  }

  private async pump(record: ConversationRunRecord): Promise<void> {
    const reader = record.reader
    if (!reader) return
    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        if (value.type === 'error') record.sawErrorChunk = true
        record.chunks.push(value)
        for (const subscriber of [...record.subscribers]) {
          try {
            subscriber.enqueue(value)
          } catch {
            record.subscribers.delete(subscriber)
          }
        }
      }
      await this.finish(
        record,
        record.abortController.signal.aborted
          ? 'aborted'
          : record.sawErrorChunk
            ? 'failed'
            : 'completed',
      )
    } catch {
      await this.finish(
        record,
        record.abortController.signal.aborted ? 'aborted' : 'failed',
      )
    } finally {
      reader.releaseLock()
    }
  }

  private async finish(
    record: ConversationRunRecord,
    status: Exclude<ConversationRunStatus, 'running'>,
  ): Promise<void> {
    if (record.ended) return
    record.ended = true
    record.status = status
    for (const subscriber of record.subscribers) {
      try {
        subscriber.close()
      } catch {
        // A detached HTTP response may already have closed its controller.
      }
    }
    record.subscribers.clear()
    try {
      await record.onEnd?.(status, record.runId)
    } finally {
      record.resolveFinished()
    }
  }
}
