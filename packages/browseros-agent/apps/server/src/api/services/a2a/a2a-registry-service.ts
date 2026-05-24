/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * A2A (Agent-to-Agent) Registry Service
 *
 * Hybrid storage: PostgreSQL for persistent agent cards + in-memory for
 * realtime message queuing and SSE subscriptions.
 */

import { logger } from '../../../lib/logger'
import { PgAgentStore } from './pg-agent-store'

export interface A2aAgentCard {
  id: string
  name: string
  description: string
  capabilities: string[]
  version: string
  endpoint?: string
}

export interface A2aMessage {
  id: string
  sender: string
  recipient?: string
  type: string
  payload: unknown
  timestamp: string
}

export interface A2aTask {
  id: string
  title: string
  description: string
  state: string
  priority: number
  assignee: string
  createdAt: string
  updatedAt: string
}

type MessageHandler = (message: A2aMessage) => void

export class A2aRegistryService {
  private messageQueue = new Map<string, A2aMessage[]>()
  private tasks = new Map<string, A2aTask>()
  private subscribers = new Map<string, MessageHandler>()
  private heartbeatInterval: ReturnType<typeof setInterval> | null = null
  private pg: PgAgentStore | null = null
  private pgReady = false

  constructor(pgDsn?: string) {
    if (pgDsn) {
      this.pg = new PgAgentStore(pgDsn)
      this.initPg()
    }
    this.startHeartbeatWatchdog()
  }

  private async initPg(): Promise<void> {
    try {
      await this.pg!.connect()
      await this.pg!.ensureSchema()
      this.pgReady = true
      logger.info('A2A registry PostgreSQL backend ready')
    } catch (err) {
      logger.warn(
        'A2A PostgreSQL backend failed, falling back to memory-only',
        {
          error: err instanceof Error ? err.message : String(err),
        },
      )
    }
  }

  destroy(): void {
    if (this.heartbeatInterval) {
      clearInterval(this.heartbeatInterval)
      this.heartbeatInterval = null
    }
    this.pg?.disconnect().catch(() => {})
  }

  async register(card: A2aAgentCard): Promise<void> {
    if (this.pgReady) {
      await this.pg!.upsertAgent(card, 'online')
    }
    logger.info('A2A agent registered', { agentId: card.id, name: card.name })
  }

  async unregister(agentId: string): Promise<boolean> {
    if (this.pgReady) {
      await this.pg!.markOffline(agentId)
    }
    this.subscribers.delete(agentId)
    this.messageQueue.delete(agentId)
    logger.info('A2A agent unregistered', { agentId })
    return true
  }

  async heartbeat(agentId: string): Promise<boolean> {
    if (this.pgReady) {
      const ok = await this.pg!.heartbeat(agentId)
      return ok
    }
    return false
  }

  async listAgents(): Promise<A2aAgentCard[]> {
    if (this.pgReady) {
      return this.pg!.listAgents(true)
    }
    return []
  }

  async listMatrix(): Promise<any[]> {
    if (this.pgReady) {
      return this.pg!.listMatrix()
    }
    return []
  }

  sendMessage(message: A2aMessage): boolean {
    const delivered = this.deliver(message)
    if (!delivered) {
      const queue = this.messageQueue.get(message.recipient ?? '') ?? []
      queue.push(message)
      this.messageQueue.set(message.recipient ?? '', queue)
    }
    return delivered
  }

  assignTask(task: A2aTask, agentId: string): boolean {
    task.state = 'pending'
    task.assignee = agentId
    this.tasks.set(task.id, task)

    const msg: A2aMessage = {
      id: crypto.randomUUID(),
      sender: 'system',
      recipient: agentId,
      type: 'taskAssign',
      payload: task,
      timestamp: new Date().toISOString(),
    }

    const delivered = this.deliver(msg)
    if (!delivered) {
      const queue = this.messageQueue.get(agentId) ?? []
      queue.push(msg)
      this.messageQueue.set(agentId, queue)
    }

    logger.info('A2A task assigned', {
      taskId: task.id,
      agentId,
      title: task.title,
    })
    return delivered
  }

  updateTaskState(taskId: string, state: string): boolean {
    const task = this.tasks.get(taskId)
    if (!task) return false
    task.state = state
    task.updatedAt = new Date().toISOString()

    const msg: A2aMessage = {
      id: crypto.randomUUID(),
      sender: 'system',
      recipient: task.assignee,
      type: 'taskUpdate',
      payload: task,
      timestamp: new Date().toISOString(),
    }

    this.deliver(msg)
    return true
  }

  subscribe(agentId: string, handler: MessageHandler): void {
    this.subscribers.set(agentId, handler)
    this.flushQueue(agentId)
  }

  unsubscribe(agentId: string): void {
    this.subscribers.delete(agentId)
  }

  private deliver(message: A2aMessage): boolean {
    const target = message.recipient ?? ''
    if (!target) return false

    const handler = this.subscribers.get(target)
    if (handler) {
      try {
        handler(message)
        return true
      } catch {
        return false
      }
    }
    return false
  }

  private flushQueue(agentId: string): void {
    const queue = this.messageQueue.get(agentId)
    if (!queue || queue.length === 0) return

    const handler = this.subscribers.get(agentId)
    if (!handler) return

    while (queue.length > 0) {
      const msg = queue.shift()
      if (msg) {
        try {
          handler(msg)
        } catch {
          queue.unshift(msg)
          break
        }
      }
    }

    if (queue.length === 0) {
      this.messageQueue.delete(agentId)
    }
  }

  private startHeartbeatWatchdog(): void {
    this.heartbeatInterval = setInterval(async () => {
      if (this.pgReady) {
        try {
          const stale = await this.pg!.pruneOffline(90)
          for (const agentId of stale) {
            logger.warn('A2A agent marked offline due to missed heartbeats', {
              agentId,
            })
            this.subscribers.delete(agentId)
          }
        } catch (err) {
          logger.warn('A2A heartbeat watchdog PG error', {
            error: err instanceof Error ? err.message : String(err),
          })
        }
      }
    }, 60_000)
  }
}
