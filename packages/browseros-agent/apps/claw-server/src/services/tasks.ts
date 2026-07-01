/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Read-time task deriver. A "task" is one MCP session: the dispatches
 * sharing a sessionId, plus the session-start and session-end rows for
 * that session. The deriver groups tool_dispatches by session_id at
 * the SQL layer, then walks each group in JS to compute title (first
 * tabs URL), site, status (Live / Done / Failed), and the screenshot
 * dispatch ids in chronological order.
 *
 * Status semantics:
 *   - Failed: any dispatch with result_meta.isError = 1, or an
 *     agent_session_ends row with kind = 'errored'.
 *   - Done:   agent_session_ends row with kind = 'closed', OR
 *             no end row AND last dispatch older than IDLE_TIMEOUT_MS.
 *   - Live:   otherwise (no end row AND last dispatch recent).
 *
 * Pagination: cursor is the max(id) of the last task in the previous
 * page; rows with id < cursor go into the next page. The id is
 * monotonic so this ties cleanly even when dispatches land in the
 * same millisecond.
 */

import { existsSync } from 'node:fs'
import { and, desc, eq, inArray, lt, sql } from 'drizzle-orm'
import { getAuditDb } from '../modules/db/db'
import {
  agentSessionEnds,
  agentSessionStarts,
  type ToolDispatchRow,
  toolDispatches,
} from '../modules/db/schema/schema'
import { screenshotPath } from './screenshots'

/**
 * Returns true if this dispatch actually has a screenshot file on
 * disk. Replaces the older `toolName === 'screenshot'` heuristic
 * which predated the screencast fallback + first-capture policy in
 * `persistScreenshot`: many non-screenshot-tool dispatches (navigate,
 * act, tabs new, first read on a page, ...) now produce files too.
 * A tiny `existsSync` per dispatch is cheap for the audit endpoints
 * and correct across every write path.
 */
function dispatchHasScreenshotFile(d: {
  id: number
  resultMeta: string | null
}): boolean {
  if (resultIsError(d.resultMeta)) return false
  return existsSync(screenshotPath(d.id))
}

const IDLE_TIMEOUT_MS = 5 * 60 * 1000

export type TaskStatus = 'live' | 'done' | 'failed'

export interface TaskSummary {
  sessionId: string
  agentId: string
  slug: string
  agentLabel: string
  title: string
  site: string | null
  startedAt: number
  endedAt: number | null
  durationMs: number
  dispatchCount: number
  toolSequence: string[]
  status: TaskStatus
  errorCount: number
  lastScreenshotDispatchId: number | null
  /** Highest dispatch id in this session. Used as the list cursor. */
  cursorId: number
}

export interface TaskDetail extends TaskSummary {
  dispatches: ToolDispatchRow[]
  /** Dispatch ids of every screenshot in this session, in order. */
  screenshotDispatchIds: number[]
  startEvent: {
    createdAt: number
    clientName: string
    clientVersion: string
  } | null
  endEvent: {
    createdAt: number
    kind: 'closed' | 'errored'
    reason: string | null
  } | null
}

export interface ListTasksQuery {
  agentId?: string
  status?: TaskStatus
  site?: string
  search?: string
  /** epoch ms; tasks with startedAt < since are filtered out. */
  since?: number
  /** Cursor: max dispatch id of the last task in the previous page. */
  cursor?: number
  /** Default 25, cap 100. */
  limit?: number
}

export interface ListTasksResult {
  tasks: TaskSummary[]
  nextCursor: number | null
}

export function listTasks(query: ListTasksQuery): ListTasksResult {
  const db = getAuditDb()
  const limit = Math.min(Math.max(query.limit ?? 25, 1), 100)

  const wheres = []
  if (query.agentId) wheres.push(eq(toolDispatches.agentId, query.agentId))
  if (typeof query.cursor === 'number') {
    wheres.push(lt(toolDispatches.id, query.cursor))
  }

  // 1. One row per sessionId, ordered by max dispatch id desc, paginated.
  const sessionsRaw = db
    .select({
      sessionId: toolDispatches.sessionId,
      maxId: sql<number>`max(${toolDispatches.id})`.as('max_id'),
      startedAt: sql<number>`min(${toolDispatches.createdAt})`.as('started_at'),
      lastDispatchAt: sql<number>`max(${toolDispatches.createdAt})`.as(
        'last_dispatch_at',
      ),
      dispatchCount: sql<number>`count(*)`.as('dispatch_count'),
      errorCount:
        sql<number>`sum(case when json_extract(${toolDispatches.resultMeta}, '$.isError') = 1 then 1 else 0 end)`.as(
          'error_count',
        ),
    })
    .from(toolDispatches)
    .where(wheres.length > 0 ? and(...wheres) : undefined)
    .groupBy(toolDispatches.sessionId)
    .orderBy(desc(sql`max(${toolDispatches.id})`))
    .limit(limit + 1)
    .all()

  if (sessionsRaw.length === 0) return { tasks: [], nextCursor: null }

  const pageSessions = sessionsRaw.slice(0, limit)
  const sessionIds = pageSessions.map((s) => s.sessionId)

  // 2. All dispatches for the page sessions, ordered by id.
  const allDispatches = db
    .select()
    .from(toolDispatches)
    .where(inArray(toolDispatches.sessionId, sessionIds))
    .orderBy(toolDispatches.id)
    .all()

  // 3. Session-end rows for the page sessions. orderBy(id) so that a
  //    session with multiple end rows (e.g. transport.onerror followed
  //    by onsessionclosed) resolves to the FIRST row consistently,
  //    matching getTask's `ends[0]` choice. Without this, listTasks
  //    and getTask could disagree on a task's status.
  const ends = db
    .select()
    .from(agentSessionEnds)
    .where(inArray(agentSessionEnds.sessionId, sessionIds))
    .orderBy(agentSessionEnds.id)
    .all()

  const endBySession = new Map<string, (typeof ends)[number]>()
  for (const e of ends) {
    if (!endBySession.has(e.sessionId)) endBySession.set(e.sessionId, e)
  }
  const dispatchesBySession = new Map<string, ToolDispatchRow[]>()
  for (const d of allDispatches) {
    const arr = dispatchesBySession.get(d.sessionId) ?? []
    arr.push(d)
    dispatchesBySession.set(d.sessionId, arr)
  }

  const now = Date.now()
  let tasks: TaskSummary[] = pageSessions.map((s) => {
    const ds = dispatchesBySession.get(s.sessionId) ?? []
    return buildSummary({
      sessionId: s.sessionId,
      cursorId: s.maxId,
      startedAt: s.startedAt,
      lastDispatchAt: s.lastDispatchAt,
      dispatchCount: s.dispatchCount,
      errorCount: s.errorCount,
      dispatches: ds,
      end: endBySession.get(s.sessionId) ?? null,
      now,
    })
  })

  // 4. Task-level filters that the SQL layer cannot express.
  if (query.status) tasks = tasks.filter((t) => t.status === query.status)
  if (query.site) tasks = tasks.filter((t) => t.site === query.site)
  if (typeof query.since === 'number') {
    const since = query.since
    tasks = tasks.filter((t) => t.startedAt >= since)
  }
  if (query.search) {
    const q = query.search.toLowerCase()
    tasks = tasks.filter(
      (t) =>
        t.title.toLowerCase().includes(q) ||
        t.agentLabel.toLowerCase().includes(q) ||
        (t.site?.toLowerCase().includes(q) ?? false),
    )
  }

  const nextCursor =
    sessionsRaw.length > limit
      ? (pageSessions[pageSessions.length - 1]?.maxId ?? null)
      : null

  return { tasks, nextCursor }
}

export function getTask(sessionId: string): TaskDetail | null {
  const db = getAuditDb()
  const dispatches = db
    .select()
    .from(toolDispatches)
    .where(eq(toolDispatches.sessionId, sessionId))
    .orderBy(toolDispatches.id)
    .all()
  if (dispatches.length === 0) return null

  const ends = db
    .select()
    .from(agentSessionEnds)
    .where(eq(agentSessionEnds.sessionId, sessionId))
    .orderBy(agentSessionEnds.id)
    .all()
  const starts = db
    .select()
    .from(agentSessionStarts)
    .where(eq(agentSessionStarts.sessionId, sessionId))
    .orderBy(agentSessionStarts.id)
    .all()

  const startedAt = dispatches[0]!.createdAt
  const lastDispatchAt = dispatches[dispatches.length - 1]!.createdAt
  const errorCount = dispatches.reduce(
    (n, d) => n + (resultIsError(d.resultMeta) ? 1 : 0),
    0,
  )
  const summary = buildSummary({
    sessionId,
    cursorId: dispatches[dispatches.length - 1]!.id,
    startedAt,
    lastDispatchAt,
    dispatchCount: dispatches.length,
    errorCount,
    dispatches,
    end: ends[0] ?? null,
    now: Date.now(),
  })

  return {
    ...summary,
    dispatches,
    screenshotDispatchIds: dispatches
      // Only include dispatches whose screenshot file actually
      // exists on disk. Covers the explicit `screenshot` tool, the
      // screencast fallback path (navigate / act / tabs new / ...),
      // and the first-capture override for read-only tools. Skipping
      // missing files avoids broken thumbnails in the UI strip.
      .filter(dispatchHasScreenshotFile)
      .map((d) => d.id),
    startEvent: starts[0]
      ? {
          createdAt: starts[0].createdAt,
          clientName: starts[0].clientName,
          clientVersion: starts[0].clientVersion,
        }
      : null,
    endEvent: ends[0]
      ? {
          createdAt: ends[0].createdAt,
          kind: ends[0].kind,
          reason: ends[0].reason,
        }
      : null,
  }
}

interface BuildSummaryInput {
  sessionId: string
  cursorId: number
  startedAt: number
  lastDispatchAt: number
  dispatchCount: number
  errorCount: number
  dispatches: ToolDispatchRow[]
  end: {
    createdAt: number
    kind: 'closed' | 'errored'
    reason: string | null
  } | null
  now: number
}

function buildSummary(input: BuildSummaryInput): TaskSummary {
  const ds = input.dispatches
  const first = ds[0]
  const lastScreenshot = [...ds]
    .reverse()
    // Same disk-existence check as getTask's screenshotDispatchIds
    // filter; picks the most recent dispatch whose JPEG is present
    // as the hero thumbnail on the TaskCard.
    .find(dispatchHasScreenshotFile)
  const site = firstSiteOf(ds)
  const title = site
    ? `Browsed ${site}`
    : `Session on ${first?.agentLabel ?? 'agent'}`
  const status = deriveStatus({
    end: input.end,
    errorCount: input.errorCount,
    lastDispatchAt: input.lastDispatchAt,
    now: input.now,
  })
  const endedAt = input.end?.createdAt ?? null
  const durationMs = (endedAt ?? input.lastDispatchAt) - input.startedAt

  return {
    sessionId: input.sessionId,
    agentId: first?.agentId ?? '',
    slug: first?.slug ?? '',
    agentLabel: first?.agentLabel ?? 'agent',
    title,
    site,
    startedAt: input.startedAt,
    endedAt,
    durationMs,
    dispatchCount: input.dispatchCount,
    toolSequence: ds.map((d) => d.toolName),
    status,
    errorCount: input.errorCount,
    lastScreenshotDispatchId: lastScreenshot?.id ?? null,
    cursorId: input.cursorId,
  }
}

function deriveStatus(input: {
  end: { kind: 'closed' | 'errored' } | null
  errorCount: number
  lastDispatchAt: number
  now: number
}): TaskStatus {
  if (input.end?.kind === 'errored') return 'failed'
  if (input.errorCount > 0) return 'failed'
  if (input.end?.kind === 'closed') return 'done'
  if (input.now - input.lastDispatchAt > IDLE_TIMEOUT_MS) return 'done'
  return 'live'
}

function hostnameOf(url: string): string | null {
  try {
    return new URL(url).hostname || null
  } catch {
    return null
  }
}

/**
 * Returns the first non-null hostname found across the session's
 * dispatches. The `url` column on each row is captured from
 * `session.pages.getInfo(pageId)`, which is null on tool calls that
 * do not take a `page` arg (tabs.new is the most common case). Fall
 * back to argsJson which carries the raw call args including url.
 */
function firstSiteOf(ds: ToolDispatchRow[]): string | null {
  for (const d of ds) {
    if (d.url) {
      const h = hostnameOf(d.url)
      if (h) return h
    }
  }
  for (const d of ds) {
    const url = urlFromArgs(d.argsJson)
    if (url) {
      const h = hostnameOf(url)
      if (h) return h
    }
  }
  return null
}

function urlFromArgs(argsJson: string | null): string | null {
  if (!argsJson) return null
  try {
    const parsed = JSON.parse(argsJson) as { url?: unknown }
    return typeof parsed.url === 'string' && parsed.url.length > 0
      ? parsed.url
      : null
  } catch {
    return null
  }
}

function resultIsError(resultMetaJson: string | null): boolean {
  if (!resultMetaJson) return false
  try {
    const parsed = JSON.parse(resultMetaJson) as { isError?: unknown }
    return parsed.isError === true
  } catch {
    return false
  }
}
