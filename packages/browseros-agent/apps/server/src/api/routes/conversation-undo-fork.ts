/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Conversation undo/fork routes for ACP agent sessions.
 *
 * - Undo: truncates messages from a given turn boundary, cancels any
 *   active turn, and persists the trimmed session record.
 * - Fork: copies messages up to a given turn boundary into a new
 *   session record with a fresh conversation ID.
 */

import { Hono } from 'hono'
import type { Context } from 'hono'
import type { Env } from '../types'

// ── Request/Response types ────────────────────────────────────────────

export interface UndoRequest {
  conversationId: string
  messageId: string
}

export interface ForkRequest {
  conversationId: string
  messageId: string
  newConversationId?: string
}

export interface UndoResponse {
  success: true
  remainingCount: number
}

export interface ForkResponse {
  success: true
  newConversationId: string
  messageCount: number
}

// ── Turn-boundary helpers ─────────────────────────────────────────────

type SessionMessage = unknown

/**
 * Find the array index of the user message that starts the turn
 * containing `messageIndex`. Walks backwards; skips "Resume" markers.
 *
 * A turn is a {User, Agent} pair. If `messageIndex` points at an Agent
 * message, we walk back to find the User message that started it.
 */
export function findTurnStart(
  messages: SessionMessage[],
  messageIndex: number,
): number {
  for (let i = messageIndex; i >= 0; i--) {
    const msg = messages[i]
    if (msg === 'Resume') continue
    if (typeof msg === 'object' && msg !== null && 'User' in msg) return i
  }
  // Fallback: return the requested index
  return messageIndex
}

/**
 * Find the message index by its string id (User messages carry `id`).
 * Returns -1 if not found.
 */
export function findMessageIndexById(
  messages: SessionMessage[],
  messageId: string,
): number {
  for (let i = 0; i < messages.length; i++) {
    const msg = messages[i]
    if (
      typeof msg === 'object' &&
      msg !== null &&
      'User' in msg &&
      (msg as { User: { id: string } }).User.id === messageId
    ) {
      return i
    }
  }
  return -1
}

/**
 * Truncate messages at a turn boundary. Returns the prefix up to
 * (but NOT including) the turn that contains `messageIndex`.
 */
export function truncateFromTurn(
  messages: SessionMessage[],
  messageIndex: number,
): SessionMessage[] {
  const turnStart = findTurnStart(messages, messageIndex)
  return messages.slice(0, turnStart)
}

/**
 * Copy messages up to and including the user message of a turn.
 * Used for fork: the forked conversation includes the user prompt
 * so the user can re-send or edit it.
 */
export function copyUpToTurn(
  messages: SessionMessage[],
  messageIndex: number,
): SessionMessage[] {
  const turnStart = findTurnStart(messages, messageIndex)
  return messages.slice(0, turnStart + 1)
}

// ── Route handler dependencies ────────────────────────────────────────

export interface ConversationUndoForkDeps {
  /**
   * Cancel an active turn for the agent. Used by undo to stop
   * any in-flight stream before truncating.
   */
  cancelActiveTurn?(agentId: string): boolean

  /**
   * Load the session record for an agent's main session.
   * Returns null if no session exists.
   */
  loadSessionRecord(agentId: string): Promise<SessionRecordHandle | null>
}

export interface SessionRecordHandle {
  /** Current messages array from the session record */
  messages: SessionMessage[]
  /** Persist a modified messages array back to the store */
  save(messages: SessionMessage[]): Promise<void>
  /**
   * Fork: create a new session record with the given prefix messages.
   * Optional — if not supported, the route returns 501.
   */
  fork?(
    prefixMessages: SessionMessage[],
    newId?: string,
  ): Promise<{ newConversationId: string; messageCount: number }>
}

// ── Routes ────────────────────────────────────────────────────────────

export function createConversationUndoForkRoutes(
  deps: ConversationUndoForkDeps,
) {
  return new Hono<Env>()
    .post('/:agentId/conversation/undo', async (c: Context<Env>) => {
      const agentId = c.req.param('agentId')
      const body = await readJsonBody(c)
      if ('error' in body) return c.json({ error: body.error }, 400)

      const { conversationId, messageId } = parseUndoRequest(body.value)
      if (!conversationId || !messageId) {
        return c.json(
          { error: 'conversationId and messageId are required' },
          400,
        )
      }

      const record = await deps.loadSessionRecord(agentId)
      if (!record) {
        return c.json({ error: 'Session not found' }, 404)
      }

      const msgIndex = findMessageIndexById(record.messages, messageId)
      if (msgIndex < 0) {
        return c.json({ error: 'Message not found in session' }, 404)
      }

      // Cancel any active turn before truncating
      deps.cancelActiveTurn?.(agentId)

      const truncated = truncateFromTurn(record.messages, msgIndex)
      await record.save(truncated)

      const response: UndoResponse = {
        success: true,
        remainingCount: truncated.length,
      }
      return c.json(response)
    })
    .post('/:agentId/conversation/fork', async (c: Context<Env>) => {
      const agentId = c.req.param('agentId')
      const body = await readJsonBody(c)
      if ('error' in body) return c.json({ error: body.error }, 400)

      const { conversationId, messageId, newConversationId } =
        parseForkRequest(body.value)
      if (!conversationId || !messageId) {
        return c.json(
          { error: 'conversationId and messageId are required' },
          400,
        )
      }

      const record = await deps.loadSessionRecord(agentId)
      if (!record) {
        return c.json({ error: 'Session not found' }, 404)
      }

      const msgIndex = findMessageIndexById(record.messages, messageId)
      if (msgIndex < 0) {
        return c.json({ error: 'Message not found in session' }, 404)
      }

      if (!record.fork) {
        return c.json({ error: 'Fork not supported for this session' }, 501)
      }

      const result = await record.fork(
        copyUpToTurn(record.messages, msgIndex),
        newConversationId,
      )

      const response: ForkResponse = {
        success: true,
        newConversationId: result.newConversationId,
        messageCount: result.messageCount,
      }
      return c.json(response)
    })
}

// ── Parsers ───────────────────────────────────────────────────────────

function parseUndoRequest(
  record: Record<string, unknown>,
): Partial<UndoRequest> {
  return {
    conversationId:
      typeof record.conversationId === 'string'
        ? record.conversationId
        : undefined,
    messageId:
      typeof record.messageId === 'string' ? record.messageId : undefined,
  }
}

function parseForkRequest(
  record: Record<string, unknown>,
): Partial<ForkRequest> {
  return {
    conversationId:
      typeof record.conversationId === 'string'
        ? record.conversationId
        : undefined,
    messageId:
      typeof record.messageId === 'string' ? record.messageId : undefined,
    newConversationId:
      typeof record.newConversationId === 'string'
        ? record.newConversationId
        : undefined,
  }
}

async function readJsonBody(
  c: Context<Env>,
): Promise<{ value: Record<string, unknown> } | { error: string }> {
  let body: unknown
  try {
    body = await c.req.json()
  } catch {
    return { error: 'Invalid JSON body' }
  }
  if (!body || typeof body !== 'object' || Array.isArray(body)) {
    return { error: 'JSON object body is required' }
  }
  return { value: body as Record<string, unknown> }
}
