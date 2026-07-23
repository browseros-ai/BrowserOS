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
 * ids in chronological order. The live cockpit uses the separate
 * batched summary projection, which returns grouped rows and only checks
 * screenshot files needed to expose the latest session-owned visual id.
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

import { and, desc, eq, inArray, lt, sql } from 'drizzle-orm'
import { getAuditDb } from '../modules/db/db'
import {
  agentSessionEnds,
  agentSessionStarts,
  type ToolDispatchRow,
  toolDispatches,
} from '../modules/db/schema/schema'
import { hasSessionScreenshotFile } from './screenshots'

/**
 * Screenshot metadata is advertised only after its session-owned file exists.
 * The legacy flat-file check keeps old captures readable through the new route.
 */
function dispatchHasScreenshotFile(d: {
  id: number
  sessionId: string
}): boolean {
  return hasSessionScreenshotFile(d.sessionId, d.id)
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
  latestScreenshotId: number | null
  /** Highest dispatch id in this session. Used as the list cursor. */
  cursorId: number
}

export interface TaskDetail extends TaskSummary {
  dispatches: ToolDispatchRow[]
  /** Screenshot ids in chronological order. */
  screenshotIds: number[]
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
  slug?: string
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
  if (query.slug) wheres.push(eq(toolDispatches.slug, query.slug))
  // Pagination is per-session, keyed on each session's max(id) (see orderBy /
  // nextCursor below), so the cursor must filter grouped rows via HAVING, not
  // individual rows via WHERE. Applying it as a row-level `id < cursor` before
  // GROUP BY re-groups earlier sessions that still own rows below the cursor,
  // returning them again on later pages with truncated aggregate counts.

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
    .having(
      typeof query.cursor === 'number'
        ? lt(sql`max(${toolDispatches.id})`, query.cursor)
        : undefined,
    )
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

interface TaskSummaryAggregateRow {
  sessionId: string
  cursorId: number
  startedAt: number
  lastDispatchAt: number
  dispatchCount: number
  errorCount: number
  agentId: string
  slug: string
  agentLabel: string
  toolSequenceJson: string
  urlsJson: string
  argsJsonsJson: string
}

/**
 * Reads summary projections for the requested connected sessions and resolves
 * the newest on-disk screenshot id without constructing task detail.
 */
export function getTaskSummaries(
  requestedSessionIds: readonly string[],
): ReadonlyMap<string, TaskSummary> {
  const sessionIds = [...new Set(requestedSessionIds)]
  if (sessionIds.length === 0) return new Map()

  const ids = sql.join(
    sessionIds.map((sessionId) => sql`${sessionId}`),
    sql`, `,
  )
  const db = getAuditDb()
  const rows = db.all<TaskSummaryAggregateRow>(sql`
    select
      session_id as "sessionId",
      max(id) as "cursorId",
      min(created_at) as "startedAt",
      max(created_at) as "lastDispatchAt",
      count(*) as "dispatchCount",
      sum(
        case
          when json_extract(result_meta, '$.isError') = 1 then 1
          else 0
        end
      ) as "errorCount",
      json_extract(json_group_array(agent_id), '$[0]') as "agentId",
      json_extract(json_group_array(slug), '$[0]') as "slug",
      json_extract(json_group_array(agent_label), '$[0]') as "agentLabel",
      json_group_array(tool_name) as "toolSequenceJson",
      json_group_array(url) as "urlsJson",
      json_group_array(args_json) as "argsJsonsJson"
    from (
      select
        id,
        created_at,
        agent_id,
        slug,
        agent_label,
        session_id,
        tool_name,
        url,
        args_json,
        result_meta
      from tool_dispatches
      where session_id in (${ids})
      order by id
    ) ordered_dispatches
    group by session_id
    order by min(id)
  `)
  const latestScreenshotBySession = new Map<string, number>()
  const screenshotCandidates = db
    .select({
      id: toolDispatches.id,
      sessionId: toolDispatches.sessionId,
    })
    .from(toolDispatches)
    .where(inArray(toolDispatches.sessionId, sessionIds))
    .orderBy(desc(toolDispatches.createdAt), desc(toolDispatches.id))
    .all()
  for (const candidate of screenshotCandidates) {
    if (latestScreenshotBySession.has(candidate.sessionId)) continue
    if (dispatchHasScreenshotFile(candidate)) {
      latestScreenshotBySession.set(candidate.sessionId, candidate.id)
    }
  }
  const ends = db
    .select()
    .from(agentSessionEnds)
    .where(inArray(agentSessionEnds.sessionId, sessionIds))
    .orderBy(agentSessionEnds.id)
    .all()
  const endBySession = new Map<string, (typeof ends)[number]>()
  for (const end of ends) {
    if (!endBySession.has(end.sessionId)) endBySession.set(end.sessionId, end)
  }

  const now = Date.now()
  return new Map(
    rows.map((row): [string, TaskSummary] => {
      const end = endBySession.get(row.sessionId) ?? null
      const toolSequence = parseStringArray(row.toolSequenceJson)
      const site = firstSiteOfSources(
        parseNullableStringArray(row.urlsJson),
        parseNullableStringArray(row.argsJsonsJson),
      )
      const endedAt = end?.createdAt ?? null
      const status = deriveStatus({
        end,
        errorCount: row.errorCount,
        lastDispatchAt: row.lastDispatchAt,
        now,
      })
      return [
        row.sessionId,
        {
          sessionId: row.sessionId,
          agentId: row.agentId,
          slug: row.slug,
          agentLabel: row.agentLabel,
          title: site ? `Browsed ${site}` : `Session on ${row.agentLabel}`,
          site,
          startedAt: row.startedAt,
          endedAt,
          durationMs: (endedAt ?? row.lastDispatchAt) - row.startedAt,
          dispatchCount: row.dispatchCount,
          toolSequence,
          status,
          errorCount: row.errorCount,
          latestScreenshotId:
            latestScreenshotBySession.get(row.sessionId) ?? null,
          cursorId: row.cursorId,
        },
      ]
    }),
  )
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
    screenshotIds: dispatches
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
  const latestScreenshot = [...ds].reverse().find(dispatchHasScreenshotFile)
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
    latestScreenshotId: latestScreenshot?.id ?? null,
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
  return firstSiteOfSources(
    ds.map((dispatch) => dispatch.url),
    ds.map((dispatch) => dispatch.argsJson),
  )
}

function firstSiteOfSources(
  urls: readonly (string | null)[],
  argsJsons: readonly (string | null)[],
): string | null {
  for (const url of urls) {
    if (url) {
      const h = hostnameOf(url)
      if (h) return h
    }
  }
  for (const argsJson of argsJsons) {
    const url = urlFromArgs(argsJson)
    if (url) {
      const h = hostnameOf(url)
      if (h) return h
    }
  }
  return null
}

function parseStringArray(json: string): string[] {
  const values = JSON.parse(json) as unknown[]
  return values.filter((value): value is string => typeof value === 'string')
}

function parseNullableStringArray(json: string): Array<string | null> {
  const values = JSON.parse(json) as unknown[]
  return values.map((value) => (typeof value === 'string' ? value : null))
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
