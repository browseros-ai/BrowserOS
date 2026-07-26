/**
 * Session read model for the canonical API. Historical queries stay on the
 * audit store; an explicit live query starts from connected MCP identities and
 * joins durable tab ownership to one reconciled browser snapshot.
 */

import type {
  SessionBrowserTab,
  SessionList,
  SessionSummary,
} from '@browseros/claw-api'
import { hexForSlug } from '../lib/agent-tab-groups'
import type { ClientIdentity } from '../lib/mcp-session'
import type { TabActivityRecord } from '../lib/tab-activity'
import type { SessionTabRow } from '../modules/db/schema/session-tabs.sql'
import { HARNESS_TO_AGENT_ID } from './harnesses'
import type { ListTasksQuery, ListTasksResult, TaskSummary } from './tasks'

export interface SessionQuery {
  profileId?: string
  slug?: string
  status?: 'live' | 'done' | 'failed'
  site?: string
  search?: string
  since?: number
  cursor?: number
  limit?: number
}

export interface CurrentBrowserPage {
  pageId: number
  targetId: string
  tabId: number
  url: string
  title: string
}

export interface SessionQueryDependencies {
  listConnectedIdentities(): ClientIdentity[]
  getConnectedIdentity(sessionId: string): ClientIdentity | null
  listTasks(query: ListTasksQuery): ListTasksResult
  getTaskSummaries(
    sessionIds: readonly string[],
  ): ReadonlyMap<string, TaskSummary>
  listOpenSessionTabs(): SessionTabRow[]
  /** Null means reconciliation was unavailable; an empty array is authoritative. */
  listBrowserPages(): Promise<CurrentBrowserPage[] | null>
  snapshotTabActivity(): TabActivityRecord[]
}

export interface SessionQueryService {
  listSessions(query: SessionQuery): Promise<SessionList>
}

interface LiveCandidate {
  identity: ClientIdentity
  task: TaskSummary
  summary: SessionSummary
}

export function createSessionQueryService(
  deps: SessionQueryDependencies,
): SessionQueryService {
  return {
    async listSessions(query) {
      if (query.status !== 'live') return listHistoricalSessions(query, deps)

      const pages = await deps.listBrowserPages()
      const identities = deps.listConnectedIdentities()
      const tasks = deps.getTaskSummaries(
        identities.map((identity) => identity.sessionId),
      )
      const candidates = identities
        .flatMap((identity): LiveCandidate[] => {
          const task = tasks.get(identity.sessionId)
          if (!task || task.dispatchCount <= 0) return []
          return [
            {
              identity,
              task,
              summary: sessionSummaryForTask(task, identity),
            },
          ]
        })
        .filter((candidate) => matchesLiveQuery(candidate, query))

      if (candidates.length === 0) return { items: [] }

      const ownerships = pages === null ? [] : deps.listOpenSessionTabs()
      const activities = pages === null ? [] : deps.snapshotTabActivity()
      const pagesByTabId = new Map(
        (pages ?? []).map((page) => [page.tabId, page]),
      )
      const activitiesByIncarnation = activityIndex(activities)
      const connectedSessionIds = new Set(
        candidates.map((candidate) => candidate.identity.sessionId),
      )
      const ownershipsBySession = new Map<string, SessionTabRow[]>()
      for (const ownership of ownerships) {
        if (!connectedSessionIds.has(ownership.sessionId)) continue
        const current = ownershipsBySession.get(ownership.sessionId) ?? []
        current.push(ownership)
        ownershipsBySession.set(ownership.sessionId, current)
      }

      return {
        items: candidates.map(({ identity, summary }) => {
          const browserTabs = projectBrowserTabs(
            identity.sessionId,
            ownershipsBySession.get(identity.sessionId) ?? [],
            pagesByTabId,
            activitiesByIncarnation,
          )
          const harness = harnessForIdentity(identity)
          return {
            ...summary,
            ...(harness === undefined ? {} : { harness }),
            color: hexForSlug(summary.slug),
            status: 'live',
            live: {
              state: browserTabs.some((tab) => tab.activityState === 'active')
                ? 'active'
                : 'idle',
              browserTabs: browserTabs.map(
                ({ activityState: _, ...tab }) => tab,
              ),
            },
          }
        }),
      }
    },
  }
}

function listHistoricalSessions(
  query: SessionQuery,
  deps: SessionQueryDependencies,
): SessionList {
  const result = deps.listTasks({
    ...(query.slug ? { slug: query.slug } : {}),
    ...(query.status ? { status: query.status } : {}),
    ...(query.site ? { site: query.site } : {}),
    ...(query.search ? { search: query.search } : {}),
    ...(query.since !== undefined ? { since: query.since } : {}),
    ...(query.cursor !== undefined ? { cursor: query.cursor } : {}),
    ...(query.limit !== undefined ? { limit: query.limit } : {}),
  })
  const items = result.tasks.map((task) =>
    sessionSummaryForTask(task, deps.getConnectedIdentity(task.sessionId)),
  )
  const filtered = query.profileId ? [] : items
  return {
    items: filtered,
    ...(result.nextCursor === null ? {} : { nextCursor: result.nextCursor }),
  }
}

export function sessionSummaryForTask(
  task: TaskSummary,
  identity: ClientIdentity | null,
): SessionSummary {
  return {
    sessionId: task.sessionId,
    slug: task.slug,
    label: task.agentLabel,
    name: identity?.label ?? task.title,
    ...(task.site === null ? {} : { site: task.site }),
    startedAt: task.startedAt,
    ...(task.endedAt === null ? {} : { endedAt: task.endedAt }),
    durationMs: Math.max(0, task.durationMs),
    dispatchCount: task.dispatchCount,
    toolSequence: task.toolSequence,
    status: task.status,
    errorCount: task.errorCount,
    ...(task.latestScreenshotId === null
      ? {}
      : { latestScreenshotId: task.latestScreenshotId }),
  }
}

function matchesLiveQuery(
  candidate: LiveCandidate,
  query: SessionQuery,
): boolean {
  if (query.profileId) return false
  if (query.slug && candidate.summary.slug !== query.slug) return false
  if (query.site && candidate.summary.site !== query.site) return false
  if (query.since !== undefined && candidate.summary.startedAt < query.since) {
    return false
  }
  if (!query.search) return true

  const search = query.search.toLowerCase()
  return [
    candidate.task.title,
    candidate.summary.label,
    candidate.summary.name,
    candidate.summary.site,
    candidate.summary.slug,
    candidate.identity.clientName,
    candidate.identity.clientTitle,
  ].some((value) => value?.toLowerCase().includes(search) ?? false)
}

function projectBrowserTabs(
  sessionId: string,
  ownerships: SessionTabRow[],
  pagesByTabId: ReadonlyMap<number, CurrentBrowserPage>,
  activitiesByIncarnation: ReadonlyMap<string, TabActivityRecord>,
): Array<SessionBrowserTab & { activityState: 'active' | 'idle' | undefined }> {
  const tabs = ownerships.flatMap((ownership) => {
    const page = pagesByTabId.get(ownership.tabId)
    if (!page) return []
    const activity = activitiesByIncarnation.get(
      activityKey(sessionId, page.tabId, page.pageId, page.targetId),
    )
    const tab: SessionBrowserTab & {
      activityState: 'active' | 'idle' | undefined
    } = {
      browserTabId: page.tabId,
      url: page.url,
      title: page.title,
      ...(activity
        ? {
            firstActivityAt: activity.firstToolAt,
            lastActivityAt: activity.lastToolAt,
            lastToolName: activity.lastToolName,
            toolCount: activity.toolCount,
            recentTools: activity.recentTools,
          }
        : { toolCount: 0, recentTools: [] }),
      activityState: activity?.status,
    }
    return [tab]
  })

  return tabs.sort((a, b) => {
    const aFreshness = a.lastActivityAt ?? Number.NEGATIVE_INFINITY
    const bFreshness = b.lastActivityAt ?? Number.NEGATIVE_INFINITY
    return bFreshness - aFreshness || a.browserTabId - b.browserTabId
  })
}

function activityIndex(
  activities: TabActivityRecord[],
): Map<string, TabActivityRecord> {
  const result = new Map<string, TabActivityRecord>()
  for (const activity of activities) {
    const key = activityKey(
      activity.sessionId,
      activity.tabId,
      activity.pageId,
      activity.targetId,
    )
    const existing = result.get(key)
    if (!existing || existing.lastToolAt < activity.lastToolAt) {
      result.set(key, activity)
    }
  }
  return result
}

function activityKey(
  sessionId: string,
  tabId: number,
  pageId: number,
  targetId: string,
): string {
  return `${sessionId}\u0000${tabId.toString()}\u0000${pageId.toString()}\u0000${targetId}`
}

function harnessForIdentity(identity: ClientIdentity): string | undefined {
  for (const [harness, agentId] of Object.entries(HARNESS_TO_AGENT_ID)) {
    if (agentId === identity.slug) return harness
  }
  return undefined
}
