/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Bridges the audit task + rrweb replay data into the shape the
 * existing `Replay.tsx` scaffold consumes. Two concurrent queries:
 *
 *   - `useTaskDetail({sessionId})` for the dispatch trail
 *     (one ToolDispatchRow per agent action) plus session start /
 *     end events. This populates `ReplayFrame[]`, the right-side
 *     EventTimeline, and the page metadata strip (agent, status,
 *     duration).
 *   - `useReplayEvents({sessionId})` for the rrweb event stream
 *     (mounted by `ReplayViewport` into rrweb-player). The events
 *     are passed through this hook's return value so the page can
 *     pick a tabPageId and the viewport can subscribe to filtered
 *     events.
 *
 * `frames` is derived from `dispatches` here. Mapping is
 * deterministic; see `mapDispatchToFrame` for the toolName ->
 * verb/kind translation. The `t` (seconds-into-session) is
 * computed against `sessionStartMs` from the task's startEvent.
 *
 * `replay` is null while either query is loading or there is no
 * task at all (404).
 */

import { useMemo } from 'react'
import { useNavigate, useParams } from 'react-router'
import type { RunStatus } from '@/lib/status'
import {
  type TaskDetail,
  type ToolDispatchRow,
  useTaskDetail,
} from '@/modules/api/audit.hooks'
import {
  type ReplayEvent,
  type ReplayFrame,
  type ReplayKind,
  type ReplayVerb,
  useReplayEvents,
} from '@/modules/api/replay.hooks'

export interface ReplayData {
  sessionId: string
  agentLabel: string
  taskTitle: string
  harness: string
  status: RunStatus
  site: string
  startedAt: string
  /**
   * Raw session start in ms since epoch. Used by `buildTabView` to
   * translate a frame's session-relative `t` into tab-relative time.
   * `startedAt` above is the formatted date string; this is the
   * machine-readable original.
   */
  startedAtMs: number
  duration: string
  /** Stat strip displayed in the header. Strings are presentation. */
  tokens: string
  steps: string
  approvals: string
  /** Total seconds the session covers, from start to last dispatch. */
  totalSeconds: number
  frames: ReplayFrame[]
  /** Distinct tabPageIds with rrweb events. */
  tabPageIds: number[]
  /** Filter helper: events scoped to one tabPageId. */
  eventsForTab: (tabPageId: number) => ReplayEvent[]
}

// `buildTabView` and the `TabView` shape live in `./tab-view.ts` so
// tests can import them without dragging the react-query-kit hook
// graph. Re-exported here for backward-compat with existing
// callers that reach it through this module.
export {
  buildTabView,
  EMPTY_TAB_VIEW,
  type TabView,
} from './tab-view'

export interface UseReplayDataResult {
  replay: ReplayData | null
  sessionId: string
  isLoading: boolean
  navigate: ReturnType<typeof useNavigate>
}

export function useReplayData(): UseReplayDataResult {
  const { sessionId = '' } = useParams<{ sessionId: string }>()
  const navigate = useNavigate()
  const taskQuery = useTaskDetail({
    variables: { sessionId },
    enabled: sessionId.length > 0,
  })
  const eventsQuery = useReplayEvents({
    variables: { sessionId },
    enabled: sessionId.length > 0,
  })

  const replay = useMemo<ReplayData | null>(() => {
    if (!taskQuery.data) return null
    return buildReplayData(taskQuery.data, eventsQuery.data?.events ?? [])
  }, [taskQuery.data, eventsQuery.data])

  return {
    replay,
    sessionId,
    isLoading: taskQuery.isLoading || eventsQuery.isLoading,
    navigate,
  }
}

function buildReplayData(task: TaskDetail, events: ReplayEvent[]): ReplayData {
  const sessionStartMs = task.startedAt
  const lastDispatchAt = task.dispatches.length
    ? task.dispatches[task.dispatches.length - 1].createdAt
    : sessionStartMs
  const totalMs = Math.max(
    1_000,
    (task.endedAt ?? lastDispatchAt) - sessionStartMs,
  )

  const frames: ReplayFrame[] = task.dispatches.map((row) =>
    mapDispatchToFrame(row, sessionStartMs),
  )

  // Preserve first-appearance order for the tab list so per-tab
  // labels (Tab 1, Tab 2, ...) match the operator's mental
  // narrative and align with the audit view's sequential
  // numbering. The raw BrowserOS pageId is non-contiguous and not
  // useful to surface as a label.
  const tabsInOrder: number[] = []
  const eventsByTab = new Map<number, ReplayEvent[]>()
  for (const ev of events) {
    const list = eventsByTab.get(ev.tabPageId)
    if (list) {
      list.push(ev)
    } else {
      eventsByTab.set(ev.tabPageId, [ev])
      tabsInOrder.push(ev.tabPageId)
    }
  }

  return {
    sessionId: task.sessionId,
    agentLabel: task.agentLabel || task.slug,
    taskTitle: task.title,
    harness: task.startEvent?.clientName ?? 'unknown',
    status: mapTaskStatus(task.status),
    site: task.site ?? 'about:blank',
    startedAt: formatStartedAt(task.startedAt),
    startedAtMs: sessionStartMs,
    duration: formatDuration(totalMs),
    tokens: '-',
    steps: String(task.dispatchCount),
    approvals: String(countApprovals(task.dispatches)),
    totalSeconds: totalMs / 1000,
    frames,
    tabPageIds: tabsInOrder,
    eventsForTab: (id) => eventsByTab.get(id) ?? [],
  }
}

const TOOL_TO_VERB: Record<string, ReplayVerb> = {
  tabs: 'navigate',
  navigate: 'navigate',
  windows: 'navigate',
  tab_groups: 'navigate',
  snapshot: 'read',
  read: 'read',
  grep: 'read',
  diff: 'read',
  screenshot: 'read',
  act: 'click',
  upload: 'attach',
  download: 'attach',
  pdf: 'read',
  wait: 'read',
  run: 'type',
  evaluate: 'type',
}

function mapDispatchToFrame(
  row: ToolDispatchRow,
  sessionStartMs: number,
): ReplayFrame {
  const t = Math.max(0, (row.createdAt - sessionStartMs) / 1000)
  const meta = row.resultMeta ? safeParse(row.resultMeta) : null
  const isError = meta?.isError === true
  const cancellationKind = meta?.cancellationKind
  const cancelled = cancellationKind === 'cockpit.operator-cancelled'
  const kind: ReplayKind = cancelled ? 'block' : isError ? 'block' : 'action'
  const note = cancelled ? 'Cancelled' : isError ? 'Errored' : undefined
  const node = row.title || row.url || row.toolName
  const verb = TOOL_TO_VERB[row.toolName] ?? 'read'
  const caption = buildCaption(row, verb, isError, cancelled)
  return {
    t,
    kind,
    verb,
    node,
    caption,
    url: row.url,
    pageId: row.pageId,
    note,
    dispatchId: row.id,
  }
}

function buildCaption(
  row: ToolDispatchRow,
  verb: ReplayVerb,
  isError: boolean,
  cancelled: boolean,
): string {
  if (cancelled) return `${row.toolName}: cancelled by operator`
  if (isError) return `${row.toolName}: errored`
  if (verb === 'navigate' && row.url) return `Navigate to ${row.url}`
  if (row.title) return `${row.toolName}: ${row.title}`
  return row.toolName
}

function countApprovals(rows: ToolDispatchRow[]): number {
  // The PR #1392 audit row metadata can carry a cancellationKind;
  // we treat the absence of error metadata as auto-approved and
  // report 0 here until the approval gate (Phase 6) lands a real
  // count. Stat is presentational; not load-bearing.
  let n = 0
  for (const r of rows) {
    const meta = r.resultMeta ? safeParse(r.resultMeta) : null
    if (meta?.approvalRequired === true) n++
  }
  return n
}

function safeParse(json: string): Record<string, unknown> | null {
  try {
    return JSON.parse(json) as Record<string, unknown>
  } catch {
    return null
  }
}

function mapTaskStatus(status: TaskDetail['status']): RunStatus {
  if (status === 'live') return 'running'
  if (status === 'failed') return 'blocked'
  return 'done'
}

function formatStartedAt(ms: number): string {
  const d = new Date(ms)
  return d.toLocaleDateString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
  })
}

function formatDuration(ms: number): string {
  const total = Math.floor(ms / 1000)
  const m = Math.floor(total / 60)
  const s = total % 60
  return `${m}:${String(s).padStart(2, '0')}`
}
