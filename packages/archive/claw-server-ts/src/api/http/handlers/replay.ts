import type { RecordingMetadata } from '@browseros/claw-api'
import type { Handler } from 'hono'
import { identityService } from '../../../lib/mcp-session'
import { replayService } from '../../../services/replays'
import { getTask } from '../../../services/tasks'
import { apiError } from '../errors'
import type { RequestContextEnv } from '../request-context'

export interface ReplayHandlerDependencies {
  getRecording(sessionId: string): RecordingMetadata | null
  downloadRecordingEvents(sessionId: string): Promise<string | null>
}

export interface ReplayHandlers {
  metadata: Handler<RequestContextEnv>
  events: Handler<RequestContextEnv>
}

export function createReplayHandlers(
  dependencies: ReplayHandlerDependencies,
): ReplayHandlers {
  return {
    metadata: (c) => {
      const recording = dependencies.getRecording(
        c.req.param('sessionId') ?? '',
      )
      if (!recording) {
        return apiError(c, 404, 'session_not_found', 'session not found')
      }
      return c.json(recording)
    },
    events: async (c) => {
      const events = await dependencies.downloadRecordingEvents(
        c.req.param('sessionId') ?? '',
      )
      if (events === null) {
        return apiError(c, 404, 'session_not_found', 'session not found')
      }
      return c.body(events, 200, { 'content-type': 'application/x-ndjson' })
    },
  }
}

export const productionReplayHandlers = createReplayHandlers({
  getRecording(sessionId) {
    if (!knownSession(sessionId)) return null
    const metadata = replayService.getMeta(sessionId)
    return {
      hasData: metadata.exists,
      complete: metadata.complete,
      sizeBytes: metadata.sizeBytes,
      ...(metadata.firstEventAt === undefined
        ? {}
        : { firstEventAt: metadata.firstEventAt }),
      ...(metadata.lastEventAt === undefined
        ? {}
        : { lastEventAt: metadata.lastEventAt }),
      tabs: metadata.tabs.map((tab) => ({
        tabId: tab.tabId,
        complete: tab.complete,
        firstEventAt: tab.firstEventAt,
        lastEventAt: tab.lastEventAt,
        segments: tab.segments.map((segment) => ({
          documentId: segment.documentId,
          ...(segment.targetId === null ? {} : { targetId: segment.targetId }),
          firstEventAt: segment.firstEventAt,
          lastEventAt: segment.lastEventAt,
          sizeBytes: segment.sizeBytes,
          eventCount: segment.eventCount,
          hasGap: segment.hasGap,
          ...(segment.legacy ? { legacy: true } : {}),
        })),
      })),
    }
  },
  async downloadRecordingEvents(sessionId) {
    if (!knownSession(sessionId)) return null
    const events = await replayService.readSession(sessionId)
    return events.length === 0
      ? ''
      : `${events.map((event) => JSON.stringify(event)).join('\n')}\n`
  },
})

function knownSession(sessionId: string): boolean {
  return Boolean(identityService.getIdentity(sessionId) ?? getTask(sessionId))
}
