/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Per-agent FIFO queue of outbound chat messages. The user submits a
 * message via /claw/agents/:id/queue, the server holds it, and a worker
 * dispatches it through the existing chatStream path the moment the
 * agent's ClawSession status flips to idle.
 *
 * The queue lives in memory only — server restart loses pending items.
 * Persistence is a follow-up; the deliberate v1 trade-off is keeping the
 * dispatch reactive (single source of truth = ClawSession) and avoiding
 * a parallel store that could drift from the agent's actual state.
 */

import { randomUUID } from 'node:crypto'
import { logger } from '../../../lib/logger'
import type {
  AgentSessionState,
  SessionStateListener,
} from '../openclaw/claw-session'
import type { OpenClawChatContentPart } from '../openclaw/openclaw-http-client'
import type { OpenClawStreamEvent } from '../openclaw/openclaw-types'

export type QueuedItemStatus = 'queued' | 'dispatching' | 'failed'

export interface QueuedItemAttachmentPreview {
  kind: 'image' | 'file'
  mediaType: string
  name?: string
}

export interface QueuedItem {
  id: string
  agentId: string
  /** Plain text body — what we send through chatStream's `message` arg. */
  message: string
  /** Multimodal parts when attachments are present. */
  messageParts?: OpenClawChatContentPart[]
  /** Compact preview the SSE feed broadcasts; never includes data URLs. */
  attachmentsPreview: QueuedItemAttachmentPreview[]
  sessionKey?: string
  history: Array<{ role: 'user' | 'assistant'; content: string }>
  status: QueuedItemStatus
  error?: string
  createdAt: number
  startedAt?: number
}

/** Public projection sent over the SSE feed — strips heavy fields. */
export interface QueuedItemPublic {
  id: string
  status: QueuedItemStatus
  message: string
  attachmentsPreview: QueuedItemAttachmentPreview[]
  error?: string
  createdAt: number
  startedAt?: number
}

interface QueueListener {
  agentId: string
  send(items: QueuedItemPublic[]): void
}

/** A "send" delegate — wraps OpenClawService.chatStream to avoid a hard dep. */
export type ChatStreamFn = (input: {
  agentId: string
  sessionKey: string
  message: string
  history: QueuedItem['history']
  messageParts?: OpenClawChatContentPart[]
  signal?: AbortSignal
}) => Promise<ReadableStream<OpenClawStreamEvent>>

interface OutboundQueueServiceDeps {
  /** Subscribe to per-agent status transitions from the ClawSession SM. */
  onAgentStatusChange(listener: SessionStateListener): () => void
  /** Read the current ClawSession state for an agent. */
  getAgentState(agentId: string): AgentSessionState
  /**
   * Look up the agent's existing user-chat sessionKey, if any. The worker
   * uses this to keep queued sends on the same conversation thread —
   * generating a fresh UUID per queued message would orphan the prior
   * conversation by spawning a brand-new session each time.
   */
  resolveExistingSessionKey(agentId: string): string | null
  /** Send a chat — wraps OpenClawService.chatStream. */
  chatStream: ChatStreamFn
}

export class OutboundQueueService {
  private readonly queues = new Map<string, QueuedItem[]>()
  private readonly listeners = new Set<QueueListener>()
  private readonly workerInflight = new Map<string, AbortController>()
  private unsubscribe: (() => void) | null = null

  constructor(private readonly deps: OutboundQueueServiceDeps) {
    this.unsubscribe = deps.onAgentStatusChange((agentId, state) => {
      if (state.status === 'idle') void this.tryDispatch(agentId)
    })
  }

  enqueue(
    item: Omit<QueuedItem, 'id' | 'status' | 'createdAt'> & { id?: string },
  ): QueuedItem {
    // Caller-supplied ids let the browser keep its optimistic row and the
    // server snapshot reconciled on a single key — without that, SSE
    // can't dedupe the optimistic entry until the POST response lands
    // and the client learns the server-generated UUID.
    const list = this.queues.get(item.agentId) ?? []
    const id =
      item.id && !list.some((existing) => existing.id === item.id)
        ? item.id
        : randomUUID()
    const queued: QueuedItem = {
      ...item,
      id,
      status: 'queued',
      createdAt: Date.now(),
    }
    list.push(queued)
    this.queues.set(item.agentId, list)
    this.broadcast(item.agentId)
    void this.tryDispatch(item.agentId)
    return queued
  }

  cancel(
    agentId: string,
    itemId: string,
  ): { ok: true } | { ok: false; reason: 'not_found' | 'dispatching' } {
    const list = this.queues.get(agentId) ?? []
    const idx = list.findIndex((i) => i.id === itemId)
    if (idx < 0) return { ok: false, reason: 'not_found' }
    const target = list[idx]
    if (!target) return { ok: false, reason: 'not_found' }
    if (target.status === 'dispatching') {
      return { ok: false, reason: 'dispatching' }
    }
    list.splice(idx, 1)
    this.queues.set(agentId, list)
    this.broadcast(agentId)
    return { ok: true }
  }

  retry(agentId: string, itemId: string): { ok: boolean } {
    const list = this.queues.get(agentId) ?? []
    const item = list.find((i) => i.id === itemId)
    if (!item || item.status !== 'failed') return { ok: false }
    item.status = 'queued'
    item.error = undefined
    this.broadcast(agentId)
    void this.tryDispatch(agentId)
    return { ok: true }
  }

  list(agentId: string): QueuedItemPublic[] {
    const items = this.queues.get(agentId) ?? []
    return items.map(toPublic)
  }

  /** Subscribe to per-agent queue state. Sends a snapshot immediately. */
  subscribe(
    agentId: string,
    send: (items: QueuedItemPublic[]) => void,
  ): () => void {
    const listener: QueueListener = { agentId, send }
    this.listeners.add(listener)
    try {
      send(this.list(agentId))
    } catch {
      // best effort
    }
    return () => {
      this.listeners.delete(listener)
    }
  }

  private broadcast(agentId: string): void {
    const snapshot = this.list(agentId)
    for (const listener of this.listeners) {
      if (listener.agentId !== agentId) continue
      try {
        listener.send(snapshot)
      } catch {
        // ignore — broken listeners GC themselves on next subscribe attempt
      }
    }
  }

  private async tryDispatch(agentId: string): Promise<void> {
    if (this.workerInflight.has(agentId)) return
    const list = this.queues.get(agentId) ?? []
    const head = list.find((i) => i.status === 'queued')
    if (!head) return

    // Don't fire if the agent isn't actually idle yet — even if the
    // listener happened to call us early during a state transition.
    const state = this.deps.getAgentState(agentId)
    if (state.status === 'working') return

    head.status = 'dispatching'
    head.startedAt = Date.now()
    this.broadcast(agentId)

    const abort = new AbortController()
    this.workerInflight.set(agentId, abort)

    try {
      // Resolution order: explicit sessionKey on the queued item ➜
      // the agent's existing user-chat session ➜ a fresh UUID for the
      // first-ever message. This prevents the queue from inadvertently
      // splintering an active conversation into a new session.
      const targetSessionKey =
        head.sessionKey ??
        this.deps.resolveExistingSessionKey(agentId) ??
        randomUUID()
      const stream = await this.deps.chatStream({
        agentId,
        sessionKey: targetSessionKey,
        message: head.message,
        history: head.history,
        messageParts: head.messageParts,
        signal: abort.signal,
      })
      // Drain the stream to completion so the gateway run finalizes
      // properly (writes the JSONL turn, releases the run controller).
      const reader = stream.getReader()
      try {
        while (true) {
          if (abort.signal.aborted) break
          const { done } = await reader.read()
          if (done) break
        }
      } finally {
        await reader.cancel().catch(() => {})
      }
      this.removeAndBroadcast(agentId, head.id)
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      logger.warn('OutboundQueue dispatch failed', {
        agentId,
        itemId: head.id,
        error: message,
      })
      head.status = 'failed'
      head.error = message
      this.broadcast(agentId)
    } finally {
      this.workerInflight.delete(agentId)
    }

    // If anything else is still queued and the agent's still idle, drain
    // it now without waiting for the next state-change callback.
    void this.tryDispatch(agentId)
  }

  private removeAndBroadcast(agentId: string, itemId: string): void {
    const list = this.queues.get(agentId) ?? []
    this.queues.set(
      agentId,
      list.filter((i) => i.id !== itemId),
    )
    this.broadcast(agentId)
  }

  shutdown(): void {
    this.unsubscribe?.()
    this.unsubscribe = null
    for (const abort of this.workerInflight.values()) abort.abort()
    this.workerInflight.clear()
    this.listeners.clear()
    this.queues.clear()
  }
}

function toPublic(item: QueuedItem): QueuedItemPublic {
  return {
    id: item.id,
    status: item.status,
    message: item.message,
    attachmentsPreview: item.attachmentsPreview,
    error: item.error,
    createdAt: item.createdAt,
    startedAt: item.startedAt,
  }
}
