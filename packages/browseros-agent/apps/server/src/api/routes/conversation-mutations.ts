/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Server-side undo / fork routes for ACP agent sessions.
 *
 * Undo  = truncate session messages from a given turn index.
 * Fork  = copy session messages up to a turn index into a new session.
 *
 * Both operate on the acpx `AcpSessionStore` (load / save) and the
 * `AcpxRuntime` which owns the session lifecycle.
 */

import type { AcpSessionRecord } from 'acpx/runtime'
import type { Context } from 'hono'
import { z } from 'zod'
import { createAcpUIMessageStreamResponse } from '../../lib/agents/acp-ui-message-stream'
import type { AcpxRuntime } from '../../lib/agents/acpx-runtime'
import { logger } from '../../lib/logger'
import type { Env } from '../types'

// ── Schemas ────────────────────────────────────────────────────────

export const UndoRequestSchema = z.object({
  /** 0-based index into the session's `messages` array to undo from. */
  messageIndex: z.number().int().min(0),
})

export const ForkRequestSchema = z.object({
  /** 0-based index into the session's `messages` array to fork at (inclusive). */
  messageIndex: z.number().int().min(0),
  /** Optional new session ID. Auto-generated if omitted. */
  newSessionId: z.string().optional(),
})

export type UndoRequest = z.infer<typeof UndoRequestSchema>
export type ForkRequest = z.infer<typeof ForkRequestSchema>

// ── Message helpers ────────────────────────────────────────────────

type SessionMessage = AcpSessionRecord['messages'][number]

/**
 * Find the turn boundary for a given message index.
 * A "turn" = consecutive User + Agent messages.
 * Returns the index of the User message that starts the turn
 * containing the given message.
 */
export function findTurnStart(
  messages: SessionMessage[],
  messageIndex: number,
): number {
  if (messageIndex < 0 || messageIndex >= messages.length) return 0
  // Walk backwards to find the nearest User message
  for (let i = messageIndex; i >= 0; i--) {
    if (messages[i] && typeof messages[i] === 'object' && 'User' in messages[i]) {
      return i
    }
  }
  return messageIndex
}

/**
 * Find the index of the last message in the turn that starts at turnStart.
 * A turn is: User message + (optional consecutive Agent messages).
 */
export function findTurnEnd(
  messages: SessionMessage[],
  turnStart: number,
): number {
  let end = turnStart
  for (let i = turnStart + 1; i < messages.length; i++) {
    const msg = messages[i]
    // Skip "Resume" markers
    if (msg === 'Resume') continue
    // If it's an Agent message, it's still part of this turn
    if (typeof msg === 'object' && 'Agent' in msg) {
      end = i
      continue
    }
    // If we hit a User message, the turn is over
    break
  }
  return end
}

/**
 * Truncate messages from a given turn. Returns the remaining messages.
 */
export function truncateMessages(
  messages: SessionMessage[],
  messageIndex: number,
): SessionMessage[] {
  const turnStart = findTurnStart(messages, messageIndex)
  return messages.slice(0, turnStart)
}

/**
 * Copy messages up to and including a turn. Returns the copied prefix.
 */
export function copyMessagesUpTo(
  messages: SessionMessage[],
  messageIndex: number,
): SessionMessage[] {
  const turnStart = findTurnStart(messages, messageIndex)
  const turnEnd = findTurnEnd(messages, turnStart)
  return messages.slice(0, turnEnd + 1)
}

// ── Service ────────────────────────────────────────────────────────

export interface ConversationMutationsService {
  undo(input: {
    agentId: string
    sessionId: string
    messageIndex: number
  }): Promise<{ success: true; remainingCount: number }>

  fork(input: {
    agentId: string
    sessionId: string
    messageIndex: number
    newSessionId?: string
  }): Promise<{
    success: true
    newSessionId: string
    newSessionKey: string
    messageCount: number
  }>
}

export function createConversationMutationsService(deps: {
  runtime: AcpxRuntime
}): ConversationMutationsService {
  return {
    async undo(input) {
      const { agentId, sessionId, messageIndex } = input
      const record = await deps.runtime.loadSessionRecord(agentId)
      if (!record) {
        throw new Error(`Session not found for agent ${agentId}`)
      }

      const originalCount = record.messages.length
      const remaining = truncateMessages(record.messages, messageIndex)

      if (remaining.length === originalCount) {
        throw new Error('Nothing to undo — index resolves to full conversation')
      }

      record.messages = remaining
      record.lastUsedAt = new Date().toISOString()
      record.updated_at = new Date().toISOString()

      await deps.runtime.saveSessionRecord(agentId, record)

      logger.info('Conversation undo', {
        agentId,
        sessionId,
        fromIndex: messageIndex,
        originalCount,
        remainingCount: remaining.length,
      })

      return { success: true, remainingCount: remaining.length }
    },

    async fork(input) {
      const { agentId, sessionId, messageIndex, newSessionId } = input
      const record = await deps.runtime.loadSessionRecord(agentId)
      if (!record) {
        throw new Error(`Session not found for agent ${agentId}`)
      }

      const prefix = copyMessagesUpTo(record.messages, messageIndex)
      const newKey = newSessionId ?? crypto.randomUUID()
      const newSessionIdFinal = `fork-${newKey}`

      const forkedRecord: AcpSessionRecord = {
        ...record,
        acpxRecordId: crypto.randomUUID(),
        acpSessionId: newSessionIdFinal,
        messages: prefix,
        name: `${record.name ?? 'Session'} (fork)`,
        createdAt: new Date().toISOString(),
        lastUsedAt: new Date().toISOString(),
        updated_at: new Date().toISOString(),
        lastSeq: 0,
        lastRequestId: undefined,
        cumulative_token_usage: {
          input_tokens: 0,
          output_tokens: 0,
          cache_creation_input_tokens: 0,
          cache_read_input_tokens: 0,
        },
        request_token_usage: {},
        acpx: {
          ...record.acpx,
          // Store fork metadata
          forkedFrom: {
            agentId,
            sessionKey: record.acpxRecordId,
            messageIndex,
          },
        },
      }

      await deps.runtime.saveSessionRecordToFork(agentId, forkedRecord)

      logger.info('Conversation fork', {
        agentId,
        sourceSessionKey: record.acpxRecordId,
        newSessionKey: forkedRecord.acpxRecordId,
        messageCount: prefix.length,
      })

      return {
        success: true,
        newSessionId: newSessionIdFinal,
        newSessionKey: forkedRecord.acpxRecordId,
        messageCount: prefix.length,
      }
    },
  }
}

// ── Route factory ──────────────────────────────────────────────────

export function createConversationMutationRoutes(deps: {
  service: ConversationMutationsService
}) {
  const { service } = deps

  return {
    async handleUndo(c: Context<Env>, agentId: string) {
      const body = await c.req.json().catch(() => ({}))
      const parsed = UndoRequestSchema.safeParse(body)
      if (!parsed.success) {
        return c.json(
          { error: 'Invalid request', details: parsed.error.flatten() },
          400,
        )
      }

      try {
        const result = await service.undo({
          agentId,
          sessionId: 'main',
          messageIndex: parsed.data.messageIndex,
        })
        return c.json(result)
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        logger.warn('Undo failed', { agentId, error: message })
        return c.json({ error: message }, 400)
      }
    },

    async handleFork(c: Context<Env>, agentId: string) {
      const body = await c.req.json().catch(() => ({}))
      const parsed = ForkRequestSchema.safeParse(body)
      if (!parsed.success) {
        return c.json(
          { error: 'Invalid request', details: parsed.error.flatten() },
          400,
        )
      }

      try {
        const result = await service.fork({
          agentId,
          sessionId: 'main',
          messageIndex: parsed.data.messageIndex,
          newSessionId: parsed.data.newSessionId,
        })
        return c.json(result)
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        logger.warn('Fork failed', { agentId, error: message })
        return c.json({ error: message }, 400)
      }
    },
  }
}
