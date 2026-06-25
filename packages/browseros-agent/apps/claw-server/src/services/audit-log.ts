/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 audit-log write + read API. Write path is fire-and-forget so a
 * SQLite hiccup never blocks an agent. Read path is cursor-paginated
 * over `createdAt DESC` and supports filter-by-agent and
 * filter-by-session.
 */

import { and, desc, eq, lt } from 'drizzle-orm'
import { logger } from '../lib/logger'
import { getAuditDb } from '../modules/db/db'
import {
  type ToolDispatchRow,
  toolDispatches,
} from '../modules/db/schema/tool-dispatches.sql'

const ARGS_JSON_MAX = 4096

export interface RecordToolDispatchInput {
  agentId: string
  slug: string
  agentLabel: string
  sessionId: string
  toolName: string
  pageId: number | null
  targetId: string | null
  url: string | null
  title: string | null
  rawArgs: unknown
  durationMs: number
  result: {
    isError: boolean
    structuredContent: unknown
    content: unknown
  }
}

/** Fire-and-forget. Never throws. */
export function recordToolDispatch(input: RecordToolDispatchInput): void {
  try {
    const db = getAuditDb()
    db.insert(toolDispatches)
      .values({
        agentId: input.agentId,
        slug: input.slug,
        agentLabel: input.agentLabel,
        sessionId: input.sessionId,
        toolName: input.toolName,
        pageId: input.pageId,
        targetId: input.targetId,
        url: input.url,
        title: input.title,
        argsJson: truncate(safeStringify(input.rawArgs)),
        resultMeta: summariseResult(input.result),
        durationMs: input.durationMs,
      })
      .run()
  } catch (err) {
    logger.warn('audit log write failed', {
      agentId: input.agentId,
      tool: input.toolName,
      error: err instanceof Error ? err.message : String(err),
    })
  }
}

export interface ListDispatchesQuery {
  agentId?: string
  sessionId?: string
  /**
   * Pagination cursor: return rows with `id < cursor`. The autoincrement
   * id is monotonic with insertion order; using it (not createdAt) as
   * the cursor avoids ties when many dispatches land in the same
   * millisecond.
   */
  cursor?: number
  /** Default 100, cap 500. */
  limit?: number
}

export interface ListDispatchesResult {
  rows: ToolDispatchRow[]
  /** Set when more rows exist; pass back into next request's `cursor`. */
  nextCursor: number | null
}

export function listDispatches(
  query: ListDispatchesQuery,
): ListDispatchesResult {
  const db = getAuditDb()
  const limit = Math.min(Math.max(query.limit ?? 100, 1), 500)
  const wheres = []
  if (query.agentId) wheres.push(eq(toolDispatches.agentId, query.agentId))
  if (query.sessionId) {
    wheres.push(eq(toolDispatches.sessionId, query.sessionId))
  }
  if (typeof query.cursor === 'number') {
    wheres.push(lt(toolDispatches.id, query.cursor))
  }
  const rows = db
    .select()
    .from(toolDispatches)
    .where(wheres.length ? and(...wheres) : undefined)
    .orderBy(desc(toolDispatches.id))
    .limit(limit + 1)
    .all()
  if (rows.length > limit) {
    const cut = rows.slice(0, limit)
    return {
      rows: cut,
      nextCursor: cut[cut.length - 1]?.id ?? null,
    }
  }
  return { rows, nextCursor: null }
}

function safeStringify(v: unknown): string {
  try {
    return JSON.stringify(v ?? {})
  } catch {
    return '"<unserialisable>"'
  }
}

function truncate(s: string): string {
  if (s.length <= ARGS_JSON_MAX) return s
  return `${s.slice(0, ARGS_JSON_MAX - 1)}~`
}

function summariseResult(r: RecordToolDispatchInput['result']): string {
  const structuredKeys =
    r.structuredContent && typeof r.structuredContent === 'object'
      ? Object.keys(r.structuredContent as Record<string, unknown>)
      : []
  const contentSummary = Array.isArray(r.content)
    ? `${(r.content as unknown[]).length} block(s)`
    : 'unknown'
  return safeStringify({
    isError: r.isError,
    contentSummary,
    structuredKeys,
  })
}
