/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { randomUUID } from 'node:crypto'
import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import { AGENT_HARNESS_LIMITS } from '@browseros/shared/constants/limits'
import { getBrowserosDir } from '../../browseros-dir'
import { logger } from '../../logger'
import type { AgentSessionId } from '../agent-types'

export interface QueuedMessageAttachment {
  mediaType: string
  data: string
}

export interface QueuedMessage {
  id: string
  createdAt: number
  sessionId?: AgentSessionId
  message: string
  attachments?: ReadonlyArray<QueuedMessageAttachment>
}

interface MessageQueueFile {
  version: 1
  queues: Record<string, QueuedMessage[]>
}

export class MessageQueueFullError extends Error {
  constructor(
    readonly agentId: string,
    readonly limit: number,
  ) {
    super(`Queue for agent ${agentId} is full (limit ${limit})`)
    this.name = 'MessageQueueFullError'
  }
}

/**
 * Per-agent durable FIFO of messages waiting to run. Persists at
 * `~/.browseros/agent-harness/message-queues.json` so queues survive
 * server restarts. Atomic temp+rename writes serialised through a
 * write lock so concurrent enqueues from different request contexts
 * don't race.
 *
 * Reads and writes always touch the whole file. The file is small in
 * practice (one short JSON record per agent, capped at 50 messages
 * each), so this keeps the implementation honest and removes any need
 * for partial-update semantics.
 */
export class FileMessageQueue {
  private readonly filePath: string
  private writeQueue: Promise<unknown> = Promise.resolve()
  private readonly maxLength: number

  constructor(options: { filePath?: string; maxLength?: number } = {}) {
    this.filePath =
      options.filePath ??
      join(getBrowserosDir(), 'agents', 'harness', 'message-queues.json')
    this.maxLength = options.maxLength ?? AGENT_HARNESS_LIMITS.QUEUE_MAX_LENGTH
  }

  async list(agentId: string): Promise<QueuedMessage[]> {
    const file = await this.read()
    return file.queues[agentId] ?? []
  }

  async snapshotAll(): Promise<Record<string, QueuedMessage[]>> {
    const file = await this.read()
    return Object.fromEntries(
      Object.entries(file.queues).map(([agentId, queue]) => [
        agentId,
        queue.slice(),
      ]),
    )
  }

  async append(
    agentId: string,
    input: {
      sessionId?: AgentSessionId
      message: string
      attachments?: ReadonlyArray<QueuedMessageAttachment>
    },
  ): Promise<QueuedMessage> {
    return this.withWriteLock(async () => {
      const file = await this.read()
      const queue = file.queues[agentId] ?? []
      if (queue.length >= this.maxLength) {
        throw new MessageQueueFullError(agentId, this.maxLength)
      }
      const queued: QueuedMessage = {
        id: randomUUID(),
        createdAt: Date.now(),
        sessionId: input.sessionId,
        message: input.message,
        attachments: input.attachments,
      }
      const next = [...queue, queued]
      await this.write({
        ...file,
        queues: { ...file.queues, [agentId]: next },
      })
      logger.info('Message queue appended', {
        agentId,
        queuedId: queued.id,
        depth: next.length,
      })
      return queued
    })
  }

  async popOldest(agentId: string): Promise<QueuedMessage | null> {
    return this.withWriteLock(async () => {
      const file = await this.read()
      const queue = file.queues[agentId] ?? []
      if (queue.length === 0) return null
      const [head, ...rest] = queue
      const nextQueues = { ...file.queues }
      if (rest.length === 0) {
        delete nextQueues[agentId]
      } else {
        nextQueues[agentId] = rest
      }
      await this.write({ ...file, queues: nextQueues })
      logger.info('Message queue popped', {
        agentId,
        queuedId: head.id,
        remaining: rest.length,
      })
      return head
    })
  }

  /**
   * Re-attach a message to the head of an agent's queue. Used by the
   * drain pump when starting a turn fails so the message isn't
   * silently dropped. Bypasses the length cap — `pushFront` is a
   * recovery primitive, not a regular append.
   */
  async pushFront(agentId: string, message: QueuedMessage): Promise<void> {
    await this.withWriteLock(async () => {
      const file = await this.read()
      const queue = file.queues[agentId] ?? []
      const next = [message, ...queue]
      await this.write({
        ...file,
        queues: { ...file.queues, [agentId]: next },
      })
      logger.info('Message queue requeued at head', {
        agentId,
        queuedId: message.id,
        depth: next.length,
      })
    })
  }

  async remove(agentId: string, messageId: string): Promise<boolean> {
    return this.withWriteLock(async () => {
      const file = await this.read()
      const queue = file.queues[agentId] ?? []
      const next = queue.filter((entry) => entry.id !== messageId)
      if (next.length === queue.length) return false
      const nextQueues = { ...file.queues }
      if (next.length === 0) {
        delete nextQueues[agentId]
      } else {
        nextQueues[agentId] = next
      }
      await this.write({ ...file, queues: nextQueues })
      logger.info('Message queue removed', { agentId, messageId })
      return true
    })
  }

  /** Agent ids that have at least one queued message. */
  async agentsWithPendingMessages(): Promise<string[]> {
    const file = await this.read()
    return Object.entries(file.queues)
      .filter(([, queue]) => queue.length > 0)
      .map(([agentId]) => agentId)
  }

  private async read(): Promise<MessageQueueFile> {
    try {
      const raw = await readFile(this.filePath, 'utf8')
      const parsed = JSON.parse(raw) as MessageQueueFile
      if (parsed.version !== 1 || typeof parsed.queues !== 'object') {
        return emptyQueueFile()
      }
      return parsed
    } catch (err) {
      if (isNotFoundError(err)) return emptyQueueFile()
      throw err
    }
  }

  private async write(file: MessageQueueFile): Promise<void> {
    await mkdir(dirname(this.filePath), { recursive: true })
    const tmpPath = `${this.filePath}.${process.pid}.${Date.now()}.tmp`
    await writeFile(tmpPath, `${JSON.stringify(file, null, 2)}\n`, 'utf8')
    await rename(tmpPath, this.filePath)
  }

  private withWriteLock<T>(fn: () => Promise<T>): Promise<T> {
    const result = this.writeQueue.then(fn, fn)
    this.writeQueue = result.then(
      () => undefined,
      () => undefined,
    )
    return result
  }
}

function emptyQueueFile(): MessageQueueFile {
  return { version: 1, queues: {} }
}

function isNotFoundError(err: unknown): boolean {
  return (
    typeof err === 'object' &&
    err !== null &&
    'code' in err &&
    err.code === 'ENOENT'
  )
}
