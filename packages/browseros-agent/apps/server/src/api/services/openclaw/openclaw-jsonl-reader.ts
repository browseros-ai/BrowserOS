/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'

// ---------------------------------------------------------------------------
// Types for raw JSONL line parsing (matches OpenClaw's internal format)
// ---------------------------------------------------------------------------

interface PiContentBlock {
  type: string
  text?: string
  id?: string
  name?: string
  arguments?: Record<string, unknown>
}

interface PiMessage {
  role?: 'user' | 'assistant' | 'toolResult'
  content?: PiContentBlock[]
  stopReason?: string
  errorMessage?: string
  usage?: {
    input?: number
    output?: number
    cost?: {
      total?: number
    }
  }
  model?: string
  provider?: string
  toolCallId?: string
  toolName?: string
  isError?: boolean
}

interface PiLine {
  type: string
  id?: string
  timestamp?: string
  message?: PiMessage
  provider?: string
  modelId?: string
  thinkingLevel?: string
  summary?: string
  firstKeptEntryId?: string
  tokensBefore?: number
}

interface SessionsJsonEntry {
  sessionId?: string
  updatedAt?: number
  [k: string]: unknown
}

type SessionsJson = Record<string, SessionsJsonEntry>

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

export type ClawEventType =
  | 'user.message'
  | 'agent.message'
  | 'agent.thinking'
  | 'agent.tool_use'
  | 'agent.tool_result'
  | 'session.model_change'
  | 'session.thinking_level_change'
  | 'session.compaction'

export interface ClawEvent {
  eventId: string
  type: ClawEventType
  content: string
  createdAt: number
  tokensIn?: number
  tokensOut?: number
  costUsd?: number
  model?: string
  toolName?: string
  toolCallId?: string
  toolArguments?: Record<string, unknown>
  isError?: boolean
}

export interface JsonlSessionEntry {
  key: string
  sessionId: string
  updatedAt: number
}

export interface JsonlSessionStats {
  userTurns: number
  assistantMessages: number
  toolCalls: number
  totalCostUsd: number
  totalTokensIn: number
  totalTokensOut: number
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

/**
 * Reads OpenClaw's per-session JSONL files directly from the host filesystem.
 * OpenClaw is the sole writer — this reader never modifies the files.
 *
 * Path layout on the host (via Lima virtiofs mount):
 *   <stateRoot>/agents/<agentId>/sessions/sessions.json
 *   <stateRoot>/agents/<agentId>/sessions/<piSessionId>.jsonl
 */
export class OpenClawJsonlReader {
  constructor(private readonly stateRoot: string) {}

  /** List all sessions for an agent by reading sessions.json. */
  listSessions(agentId: string): JsonlSessionEntry[] {
    const sessionsJson = this.readSessionsJson(agentId)
    if (!sessionsJson) return []

    const entries: JsonlSessionEntry[] = []
    for (const [key, entry] of Object.entries(sessionsJson)) {
      if (typeof entry.sessionId === 'string') {
        entries.push({
          key,
          sessionId: entry.sessionId,
          updatedAt: typeof entry.updatedAt === 'number' ? entry.updatedAt : 0,
        })
      }
    }
    return entries.sort((a, b) => b.updatedAt - a.updatedAt)
  }

  /** List all agent IDs by scanning the agents directory. */
  listAgents(): string[] {
    try {
      const entries = readdirSync(this.safePath('agents'), {
        withFileTypes: true,
      })
      return entries.filter((e) => e.isDirectory()).map((e) => e.name)
    } catch {
      return []
    }
  }

  /** Read and parse all events from a session's JSONL file. */
  listBySession(agentId: string, sessionKey: string): ClawEvent[] {
    const piSessionId = this.resolvePiSessionId(agentId, sessionKey)
    if (!piSessionId) return []

    const filePath = this.jsonlPath(agentId, piSessionId)
    let raw: string
    try {
      raw = readFileSync(filePath, 'utf8')
    } catch {
      return []
    }

    const events: ClawEvent[] = []
    for (const line of raw.split('\n')) {
      if (!line.trim()) continue
      let parsed: PiLine
      try {
        parsed = JSON.parse(line) as PiLine
      } catch {
        // Skip malformed lines — a partial line at the tail is possible
        // if OpenClaw is mid-write.
        continue
      }
      for (const event of mapLineToEvents(parsed)) {
        events.push(event)
      }
    }
    return events
  }

  /** Get the latest assistant message from a session. */
  latestAgentMessage(
    agentId: string,
    sessionKey: string,
  ): ClawEvent | undefined {
    const events = this.listBySession(agentId, sessionKey)
    for (let i = events.length - 1; i >= 0; i--) {
      if (events[i]?.type === 'agent.message') return events[i]
    }
    return undefined
  }

  /** Count user turns in a session. */
  countUserTurns(agentId: string, sessionKey: string): number {
    const events = this.listBySession(agentId, sessionKey)
    let n = 0
    for (const e of events) {
      if (e.type === 'user.message') n++
    }
    return n
  }

  /** Aggregate stats for a session. */
  getSessionStats(agentId: string, sessionKey: string): JsonlSessionStats {
    const events = this.listBySession(agentId, sessionKey)
    const stats: JsonlSessionStats = {
      userTurns: 0,
      assistantMessages: 0,
      toolCalls: 0,
      totalCostUsd: 0,
      totalTokensIn: 0,
      totalTokensOut: 0,
    }
    for (const e of events) {
      if (e.type === 'user.message') stats.userTurns++
      if (e.type === 'agent.message') {
        stats.assistantMessages++
        if (e.costUsd) stats.totalCostUsd += e.costUsd
        if (e.tokensIn) stats.totalTokensIn += e.tokensIn
        if (e.tokensOut) stats.totalTokensOut += e.tokensOut
      }
      if (e.type === 'agent.tool_use') stats.toolCalls++
    }
    return stats
  }

  // ── Private helpers ─────────────────────────────────────────────────

  /**
   * Ensure a resolved path stays within stateRoot to prevent path traversal
   * via crafted agentId or sessionId values containing ".." segments.
   */
  private safePath(...segments: string[]): string {
    const resolved = resolve(this.stateRoot, ...segments)
    const root = resolve(this.stateRoot)
    if (!resolved.startsWith(`${root}/`) && resolved !== root) {
      throw new Error(`Path traversal blocked: ${segments.join('/')}`)
    }
    return resolved
  }

  private readSessionsJson(agentId: string): SessionsJson | null {
    const filePath = this.safePath(
      'agents',
      agentId,
      'sessions',
      'sessions.json',
    )
    try {
      const raw = readFileSync(filePath, 'utf8')
      return JSON.parse(raw) as SessionsJson
    } catch {
      return null
    }
  }

  private resolvePiSessionId(
    agentId: string,
    sessionKey: string,
  ): string | undefined {
    const sessionsJson = this.readSessionsJson(agentId)
    if (!sessionsJson) return undefined

    // Try exact key match first
    const entry = sessionsJson[sessionKey]
    if (entry && typeof entry.sessionId === 'string') {
      return entry.sessionId
    }

    // Try matching by scanning all keys (handles key format variations)
    for (const [key, value] of Object.entries(sessionsJson)) {
      if (key === sessionKey || key.endsWith(`:${sessionKey}`)) {
        if (typeof value.sessionId === 'string') return value.sessionId
      }
    }

    return undefined
  }

  private jsonlPath(agentId: string, piSessionId: string): string {
    return this.safePath('agents', agentId, 'sessions', `${piSessionId}.jsonl`)
  }
}

// ---------------------------------------------------------------------------
// JSONL line → ClawEvent mapping
// ---------------------------------------------------------------------------

function mapLineToEvents(line: PiLine): ClawEvent[] {
  const eventId = line.id ?? ''
  const createdAt = line.timestamp ? Date.parse(line.timestamp) : Date.now()

  if (line.type === 'model_change') {
    const model = combineModel(line.provider, line.modelId)
    if (!model) return []
    return [
      {
        eventId,
        type: 'session.model_change',
        content: model,
        createdAt,
        model,
      },
    ]
  }

  if (line.type === 'thinking_level_change') {
    return [
      {
        eventId,
        type: 'session.thinking_level_change',
        content: line.thinkingLevel ?? 'unknown',
        createdAt,
      },
    ]
  }

  if (line.type === 'compaction') {
    return [
      {
        eventId,
        type: 'session.compaction',
        content: line.summary ?? '(compacted)',
        createdAt,
      },
    ]
  }

  if (line.type !== 'message' || !line.message) return []

  return mapMessageToEvents(line.message, eventId, createdAt)
}

function mapMessageToEvents(
  msg: PiMessage,
  eventId: string,
  createdAt: number,
): ClawEvent[] {
  if (msg.role === 'user') {
    const text = extractText(msg.content)
    if (!text) return []
    return [{ eventId, type: 'user.message', content: text, createdAt }]
  }

  if (msg.role === 'assistant') {
    return mapAssistantMessage(msg, eventId, createdAt)
  }

  if (msg.role === 'toolResult') {
    const text = extractText(msg.content)
    return [
      {
        eventId,
        type: 'agent.tool_result',
        content: text || '(no output)',
        createdAt,
        toolName: msg.toolName,
        toolCallId: msg.toolCallId,
        isError: msg.isError,
      },
    ]
  }

  return []
}

function mapAssistantMessage(
  msg: PiMessage,
  eventId: string,
  createdAt: number,
): ClawEvent[] {
  const events: ClawEvent[] = []
  const text = extractText(msg.content)

  if (msg.content) {
    let thinkingIdx = 0
    let toolIdx = 0
    for (const block of msg.content) {
      if (
        block.type === 'thinking' &&
        typeof block.text === 'string' &&
        block.text.length > 0
      ) {
        events.push({
          eventId: `${eventId}:thinking:${thinkingIdx}`,
          type: 'agent.thinking',
          content: block.text,
          createdAt,
        })
        thinkingIdx++
      }
      if (block.type === 'toolCall' && block.name) {
        events.push({
          eventId: `${eventId}:tool:${block.id ?? toolIdx}`,
          type: 'agent.tool_use',
          content: block.name,
          createdAt,
          toolName: block.name,
          toolCallId: block.id,
          toolArguments: block.arguments,
        })
        toolIdx++
      }
    }
  }

  if (text) {
    events.push({
      eventId,
      type: 'agent.message',
      content: text,
      createdAt,
      tokensIn: msg.usage?.input,
      tokensOut: msg.usage?.output,
      costUsd: msg.usage?.cost?.total,
      model: combineModel(msg.provider, msg.model),
    })
  }

  return events
}

function extractText(blocks: PiContentBlock[] | undefined): string {
  if (!blocks || blocks.length === 0) return ''
  const parts: string[] = []
  for (const block of blocks) {
    if (block.type === 'text' && typeof block.text === 'string') {
      parts.push(block.text)
    }
  }
  return parts.join('')
}

function combineModel(
  provider: string | undefined,
  model: string | undefined,
): string | undefined {
  if (!model) return undefined
  return provider ? `${provider}/${model}` : model
}
