import type { Dispatch, SessionDetail, SessionList } from '@browseros/claw-api'
import type { Handler } from 'hono'
import { getBrowserSession } from '../../../lib/browser-session'
import { logger } from '../../../lib/logger'
import { identityService } from '../../../lib/mcp-session'
import { tabActivityRegistry } from '../../../lib/tab-activity'
import { dispatchCancellation } from '../../../services/dispatch-cancellation'
import {
  createSessionQueryService,
  sessionSummaryForTask,
} from '../../../services/session-query'
import { listOpenSessionTabs } from '../../../services/session-tabs'
import {
  getTask,
  getTaskSummaries,
  listTasks,
  type TaskDetail,
} from '../../../services/tasks'
import { apiError } from '../errors'
import type { RequestContextEnv } from '../request-context'
import { optionalInteger } from '../validation'

type Awaitable<T> = T | Promise<T>

/** Liveness distinguishes cancellable transports from ended audit history and unknown ids. */
export type SessionState = 'live' | 'ended' | 'missing'

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

export interface SessionHandlerDependencies {
  listSessions(query: SessionQuery): Awaitable<SessionList>
  getSession(sessionId: string): SessionDetail | null
  getSessionState(sessionId: string): SessionState
  cancelSession(sessionId: string): number
}

export interface SessionHandlers {
  list: Handler<RequestContextEnv>
  detail: Handler<RequestContextEnv>
  cancel: Handler<RequestContextEnv>
}

export function createSessionHandlers(
  dependencies: SessionHandlerDependencies,
): SessionHandlers {
  return {
    list: async (c) => {
      const query = parseSessionQuery(c.req.query())
      if ('error' in query) {
        return apiError(c, 400, 'invalid_request', query.error)
      }
      return c.json(await dependencies.listSessions(query))
    },
    detail: (c) => {
      const session = dependencies.getSession(c.req.param('sessionId') ?? '')
      if (!session) {
        return apiError(c, 404, 'session_not_found', 'session not found')
      }
      return c.json(session)
    },
    cancel: (c) => {
      const sessionId = c.req.param('sessionId') ?? ''
      const state = dependencies.getSessionState(sessionId)
      if (state === 'missing') {
        return apiError(c, 404, 'session_not_found', 'session not found')
      }
      if (state === 'ended') {
        return apiError(c, 409, 'session_not_live', 'session is not live')
      }
      return c.json({ cancelled: dependencies.cancelSession(sessionId) })
    },
  }
}

const sessionQueryService = createSessionQueryService({
  listConnectedIdentities: () => identityService.list(),
  getConnectedIdentity: (sessionId) => identityService.getIdentity(sessionId),
  listTasks,
  getTaskSummaries,
  listOpenSessionTabs,
  async listBrowserPages() {
    const session = getBrowserSession()
    if (!session) return null
    try {
      return await session.pages.list()
    } catch (error) {
      logger.warn('live session browser reconciliation unavailable', {
        error: error instanceof Error ? error.message : String(error),
      })
      return null
    }
  },
  snapshotTabActivity: () => tabActivityRegistry.snapshot(),
})

export const productionSessionHandlers = createSessionHandlers({
  listSessions: (query) => sessionQueryService.listSessions(query),
  getSession(sessionId) {
    const task = getTask(sessionId)
    return task ? sessionDetail(task) : null
  },
  getSessionState(sessionId) {
    if (identityService.getIdentity(sessionId)) return 'live'
    return getTask(sessionId) ? 'ended' : 'missing'
  },
  cancelSession: (sessionId) =>
    dispatchCancellation.cancelBySession(
      sessionId,
      'Cancelled through the BrowserClaw API',
    ),
})

function sessionDetail(task: TaskDetail): SessionDetail {
  const screenshotIds = new Set(task.screenshotIds)
  return {
    session: sessionSummaryForTask(
      task,
      identityService.getIdentity(task.sessionId),
    ),
    dispatches: task.dispatches.map((row): Dispatch => {
      return {
        dispatchId: row.id,
        createdAt: row.createdAt,
        slug: row.slug,
        label: row.agentLabel,
        sessionId: row.sessionId,
        toolName: row.toolName,
        ...(row.pageId === null ? {} : { pageId: row.pageId }),
        ...(row.tabId === null ? {} : { tabId: row.tabId }),
        ...(row.targetId === null ? {} : { targetId: row.targetId }),
        ...(row.url === null ? {} : { url: row.url }),
        ...(row.title === null ? {} : { title: row.title }),
        ...(row.argsJson === null ? {} : { argsJson: row.argsJson }),
        ...(row.resultMeta === null ? {} : { resultMeta: row.resultMeta }),
        ...(row.durationMs === null ? {} : { durationMs: row.durationMs }),
        ...(screenshotIds.has(row.id) ? { screenshotId: row.id } : {}),
      }
    }),
  }
}

function parseSessionQuery(
  query: Record<string, string>,
): SessionQuery | { error: string } {
  const rawStatus = query.status
  if (
    rawStatus &&
    rawStatus !== 'live' &&
    rawStatus !== 'done' &&
    rawStatus !== 'failed'
  ) {
    return { error: 'status must be live, done, or failed' }
  }
  const status = rawStatus as SessionQuery['status']
  const since = optionalInteger(query.since, 0)
  if (since === false) return { error: 'since must be a non-negative integer' }
  const cursor = optionalInteger(query.cursor, 1)
  if (cursor === false) return { error: 'cursor must be a positive integer' }
  const limit = optionalInteger(query.limit, 1, 100)
  if (limit === false) return { error: 'limit must be between 1 and 100' }
  return {
    ...(query.profileId ? { profileId: query.profileId } : {}),
    ...(query.slug ? { slug: query.slug } : {}),
    ...(status ? { status } : {}),
    ...(query.site ? { site: query.site } : {}),
    ...(query.search ? { search: query.search } : {}),
    ...(typeof since === 'number' ? { since } : {}),
    ...(typeof cursor === 'number' ? { cursor } : {}),
    ...(typeof limit === 'number' ? { limit } : {}),
  }
}
