/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { UIMessage } from 'ai'
import { desc, eq } from 'drizzle-orm'
import { type BrowserOsDatabase, getDb } from '../db'
import { type ConversationRow, conversations } from '../db/schema'
import { logger } from '../logger'

export type ConversationTargetType = 'browseros' | 'claude' | 'codex'

/** Row metadata without the message blob, for cheap history listing. */
export interface ConversationSummary {
  id: string
  lastUserMessage?: string
  origin?: string
  targetType: ConversationTargetType
  agentId?: string
  lastMessagedAt: number
  createdAt: number
  updatedAt: number
}

export interface ConversationDetail extends ConversationSummary {
  messages: UIMessage[]
}

export interface SaveConversationInput {
  id: string
  messages: UIMessage[]
  targetType: ConversationTargetType
  origin?: string
  agentId?: string
  /** Preserve an original timestamp when importing legacy conversations. */
  lastMessagedAt?: number
}

export interface ConversationStore {
  list(): Promise<ConversationSummary[]>
  get(id: string): Promise<ConversationDetail | null>
  save(input: SaveConversationInput): Promise<ConversationSummary>
  /**
   * Insert only when the conversation is absent; returns null if a row already
   * exists. Used for legacy import so a newer server row is never overwritten.
   */
  insertIfAbsent(
    input: SaveConversationInput,
  ): Promise<ConversationSummary | null>
  delete(id: string): Promise<boolean>
}

const SUMMARY_COLUMNS = {
  id: conversations.id,
  lastUserMessage: conversations.lastUserMessage,
  origin: conversations.origin,
  targetType: conversations.targetType,
  agentId: conversations.agentId,
  lastMessagedAt: conversations.lastMessagedAt,
  createdAt: conversations.createdAt,
  updatedAt: conversations.updatedAt,
} as const

const MAX_SNIPPET_CHARS = 200

export class DbConversationStore implements ConversationStore {
  private readonly injectedDb?: BrowserOsDatabase
  private writeQueue: Promise<unknown> = Promise.resolve()

  constructor(options: { db?: BrowserOsDatabase } = {}) {
    this.injectedDb = options.db
  }

  // Resolve the shared handle lazily: routes (and this store) are constructed at
  // registration time, which can precede initializeDb().
  private get db(): BrowserOsDatabase {
    return this.injectedDb ?? getDb()
  }

  async list(): Promise<ConversationSummary[]> {
    return this.db
      .select(SUMMARY_COLUMNS)
      .from(conversations)
      .orderBy(desc(conversations.lastMessagedAt))
      .all()
      .map(toSummary)
  }

  async get(id: string): Promise<ConversationDetail | null> {
    const row =
      this.db
        .select()
        .from(conversations)
        .where(eq(conversations.id, id))
        .get() ?? null
    return row ? toDetail(row) : null
  }

  async save(input: SaveConversationInput): Promise<ConversationSummary> {
    return this.withWriteLock(async () => this.upsert(input, input.messages))
  }

  async insertIfAbsent(
    input: SaveConversationInput,
  ): Promise<ConversationSummary | null> {
    return this.withWriteLock(async () => {
      const existing = this.db
        .select({ id: conversations.id })
        .from(conversations)
        .where(eq(conversations.id, input.id))
        .get()
      if (existing) return null
      return this.upsert(input, input.messages)
    })
  }

  async delete(id: string): Promise<boolean> {
    return this.withWriteLock(async () => {
      const existing = this.db
        .select({ id: conversations.id })
        .from(conversations)
        .where(eq(conversations.id, id))
        .get()
      if (!existing) return false
      this.db.delete(conversations).where(eq(conversations.id, id)).run()
      logger.info('Conversation deleted', { conversationId: id })
      return true
    })
  }

  private upsert(
    input: SaveConversationInput,
    messages: UIMessage[],
  ): ConversationSummary {
    const now = Date.now()
    const existing = this.db
      .select({ createdAt: conversations.createdAt })
      .from(conversations)
      .where(eq(conversations.id, input.id))
      .get()
    const row: ConversationRow = {
      id: input.id,
      messages,
      lastUserMessage: extractLastUserText(messages) ?? null,
      origin: input.origin ?? null,
      targetType: input.targetType,
      agentId: input.agentId ?? null,
      lastMessagedAt: input.lastMessagedAt ?? now,
      createdAt: existing?.createdAt ?? now,
      updatedAt: now,
    }
    this.db
      .insert(conversations)
      .values(row)
      .onConflictDoUpdate({
        target: conversations.id,
        set: {
          messages: row.messages,
          lastUserMessage: row.lastUserMessage,
          origin: row.origin,
          targetType: row.targetType,
          agentId: row.agentId,
          lastMessagedAt: row.lastMessagedAt,
          updatedAt: row.updatedAt,
        },
      })
      .run()
    return toSummary(row)
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

type SummaryFields = Omit<ConversationRow, 'messages'>

function toSummary(row: SummaryFields): ConversationSummary {
  return {
    id: row.id,
    lastUserMessage: row.lastUserMessage ?? undefined,
    origin: row.origin ?? undefined,
    targetType: row.targetType,
    agentId: row.agentId ?? undefined,
    lastMessagedAt: row.lastMessagedAt,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  }
}

function toDetail(row: ConversationRow): ConversationDetail {
  return { ...toSummary(row), messages: row.messages }
}

function extractLastUserText(messages: UIMessage[]): string | undefined {
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index]
    if (message.role !== 'user') continue
    const text = message.parts
      .map((part) => (part.type === 'text' ? part.text : ''))
      .join(' ')
      .trim()
    if (text) return text.slice(0, MAX_SNIPPET_CHARS)
  }
  return undefined
}
