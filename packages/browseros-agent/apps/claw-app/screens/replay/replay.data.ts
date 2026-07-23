/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { RecordingMetadata } from '@browseros/claw-api'
import { useEffect, useMemo, useRef } from 'react'
import { useNavigate, useParams } from 'react-router'
import type { RunStatus } from '@/lib/status'
import {
  type TaskDetail,
  type ToolDispatchRow,
  useSessionDetail,
} from '@/modules/api/audit.hooks'
import {
  type ReplayEvent,
  type ReplayFrame,
  type ReplayKind,
  type ReplayVerb,
  replayEventsRevision,
  useReplayEvents,
  useReplayMetadata,
} from '@/modules/api/replay.hooks'
import {
  buildReplayDocumentIds,
  buildReplayEventCatalog,
  buildReplayTabIds,
  EMPTY_REPLAY_EVENTS,
  type ReplayEventCatalog,
} from './replay-events'

export interface ReplaySegmentData {
  documentId: string
  targetId?: string | null
  firstEventAt: number
  lastEventAt: number
  hasGap: boolean
  legacy: boolean
}

export interface ReplayTabData {
  tabId: number
  complete: boolean | null
  segments: ReplaySegmentData[]
}

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
  /** Total seconds the session covers, from start to last dispatch. */
  totalSeconds: number
  frames: ReplayFrame[]
  complete: boolean | null
  tabs: ReplayTabData[]
  eventsForTab: (tabId: number) => readonly ReplayEvent[]
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

/** Loads task metadata and stable rrweb event buckets for the replay page. */
export function useReplayData(): UseReplayDataResult {
  const { sessionId = '' } = useParams<{ sessionId: string }>()
  const navigate = useNavigate()
  const taskQuery = useSessionDetail({
    variables: { sessionId },
    enabled: sessionId.length > 0,
  })
  const metadataQuery = useReplayMetadata({
    variables: { sessionId },
    enabled: sessionId.length > 0,
  })
  const eventsQuery = useReplayEvents({
    variables: { sessionId },
    enabled: sessionId.length > 0,
  })
  const metadataRevision = replayEventsRevision(metadataQuery.data)
  const requestedRevision = useRef<string | null>(null)
  useEffect(() => {
    if (metadataRevision === null) return
    const sessionRevision = `${sessionId}:${metadataRevision}`
    if (requestedRevision.current === sessionRevision) return
    requestedRevision.current = sessionRevision
    void eventsQuery.refetch()
  }, [eventsQuery.refetch, metadataRevision, sessionId])
  const events = eventsQuery.data?.events ?? EMPTY_REPLAY_EVENTS
  const eventCatalog = useMemo(() => buildReplayEventCatalog(events), [events])
  const replay = useMemo<ReplayData | null>(() => {
    if (!taskQuery.data) return null
    return buildReplayData(taskQuery.data, eventCatalog, metadataQuery.data)
  }, [taskQuery.data, eventCatalog, metadataQuery.data])

  return {
    replay,
    sessionId,
    isLoading: taskQuery.isLoading,
    navigate,
  }
}

/** Converts task rows into replay metadata while reusing event buckets. */
function buildReplayData(
  detail: TaskDetail,
  eventCatalog: ReplayEventCatalog,
  metadata: RecordingMetadata | undefined,
): ReplayData {
  const { session: task, dispatches } = detail
  const sessionStartMs = task.startedAt
  const lastDispatchAt = dispatches.length
    ? dispatches[dispatches.length - 1].createdAt
    : sessionStartMs
  const totalMs = Math.max(
    1_000,
    (task.endedAt ?? lastDispatchAt) - sessionStartMs,
  )

  const tabs = buildReplayTabs(metadata, eventCatalog)
  const targetTabs = new Map<string, number>()
  for (const tab of tabs) {
    for (const segment of tab.segments) {
      if (segment.targetId) targetTabs.set(segment.targetId, tab.tabId)
    }
  }
  const frames: ReplayFrame[] = dispatches.map((row) =>
    mapDispatchToFrame(row, sessionStartMs, targetTabs),
  )

  return {
    sessionId: task.sessionId,
    agentLabel: task.label || task.slug,
    taskTitle: task.name,
    harness: task.profileId ?? 'unknown',
    status: mapTaskStatus(task.status),
    site: task.site ?? 'about:blank',
    startedAt: formatStartedAt(task.startedAt),
    startedAtMs: sessionStartMs,
    duration: formatDuration(totalMs),
    tokens: '-',
    steps: String(task.dispatchCount),
    totalSeconds: totalMs / 1000,
    frames,
    complete: metadata?.complete ?? null,
    tabs,
    eventsForTab: eventCatalog.eventsForTab,
  }
}

function buildReplayTabs(
  metadata: RecordingMetadata | undefined,
  eventCatalog: ReplayEventCatalog,
): ReplayTabData[] {
  const metadataByTab = new Map(metadata?.tabs.map((tab) => [tab.tabId, tab]))
  return buildReplayTabIds(metadata?.tabs, eventCatalog.tabIds).map((tabId) => {
    const tabMetadata = metadataByTab.get(tabId)
    const discoveredDocuments = eventCatalog.documentIdsForTab(tabId)
    const segmentMetadata = new Map(
      tabMetadata?.segments.map((segment) => [segment.documentId, segment]),
    )
    const segments = buildReplayDocumentIds(
      tabMetadata?.segments,
      discoveredDocuments,
    ).map((documentId): ReplaySegmentData => {
      const known = segmentMetadata.get(documentId)
      if (known) {
        return {
          documentId,
          targetId: known.targetId,
          firstEventAt: known.firstEventAt,
          lastEventAt: known.lastEventAt,
          hasGap: known.hasGap,
          legacy: known.legacy === true,
        }
      }
      const events = eventCatalog.eventsForDocument(documentId)
      return {
        documentId,
        targetId: events.find((event) => event.targetId)?.targetId ?? undefined,
        firstEventAt: events[0]?.ts ?? 0,
        lastEventAt: events.at(-1)?.ts ?? 0,
        hasGap: false,
        legacy: false,
      }
    })
    return {
      tabId,
      complete: tabMetadata?.complete ?? null,
      segments,
    }
  })
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
  targetTabs: ReadonlyMap<string, number>,
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
    tabId:
      row.tabId ?? (row.targetId ? targetTabs.get(row.targetId) : undefined),
    targetId: row.targetId,
    note,
    dispatchId: row.dispatchId,
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

function safeParse(json: string): Record<string, unknown> | null {
  try {
    return JSON.parse(json) as Record<string, unknown>
  } catch {
    return null
  }
}

export function mapTaskStatus(
  status: TaskDetail['session']['status'],
): RunStatus {
  if (status === 'live') return 'running'
  if (status === 'failed') return 'blocked'
  if (status === 'cancelled') return 'stopped'
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
