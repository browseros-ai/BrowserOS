/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Production wiring for the canonical API: adapts the server's existing
 * services (audit task store, MCP identity service, replay + recording
 * stores, screencast cache, harness connect) onto the
 * implementation-neutral `CanonicalApiDependencies` seam. `createServer`
 * mounts this by default; tests swap in fakes.
 *
 * Vocabulary bridge: a canonical *session* is what the audit services
 * call a *task* (both keyed by MCP session id), and a canonical
 * *dispatch* is one audited tool call.
 */

import { existsSync, readFileSync } from 'node:fs'
import type {
  Connection,
  Dispatch,
  Harness,
  SessionDetail,
} from '@browseros/claw-api'
import { RECORDING_INGEST_MAX_BYTES } from '@browseros/shared/constants/limits'
import { CLAW_API_PORT_DEFAULT } from '@browseros/shared/constants/ports'
import { getBrowserSession } from '../../lib/browser-session'
import { logger } from '../../lib/logger'
import { identityService } from '../../lib/mcp-session'
import { tabActivityRegistry } from '../../lib/tab-activity'
import { getTabTargetMap } from '../../lib/tab-targets'
import { getLocalServerUrl } from '../../local-server-url'
import {
  getTelemetryState,
  setTelemetryConsent,
} from '../../services/analytics'
import {
  type ConnectionState,
  connectBrowserosToHarness,
  disconnectBrowserosFromHarness,
  listBrowserosConnections,
} from '../../services/browseros-connect'
import { dispatchCancellation } from '../../services/dispatch-cancellation'
import {
  type RecordingEventInput,
  recordingStore,
} from '../../services/recordings'
import { replayService } from '../../services/replays'
import { screencastCache } from '../../services/screencast-cache'
import { screenshotPath } from '../../services/screenshots'
import {
  createSessionQueryService,
  sessionSummaryForTask,
} from '../../services/session-query'
import {
  getOpenSessionTab,
  listOpenSessionTabs,
} from '../../services/session-tabs'
import {
  getTask,
  getTaskSummaries,
  listTasks,
  type TaskDetail,
} from '../../services/tasks'
import { VERSION } from '../../version'
import type { CanonicalApiDependencies } from '.'

const sessionQueryService = createSessionQueryService({
  listConnectedIdentities: () => identityService.list(),
  getConnectedIdentity: (sessionId) => identityService.getIdentity(sessionId),
  listTasks,
  getTaskSummaries,
  listOpenSessionTabs,
  getOpenSessionTab,
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
  getScreencastFrame: (sessionId, pageId, targetId) =>
    screencastCache.getForSessionTarget(sessionId, pageId, targetId),
  now: () => Date.now(),
})

export const canonicalApiDependencies: CanonicalApiDependencies = {
  getSystemInfo: () => ({
    product: 'BrowserClaw',
    version: VERSION,
    url:
      getLocalServerUrl() ??
      `http://127.0.0.1:${CLAW_API_PORT_DEFAULT.toString()}`,
    capabilities: {
      recordingIngestVersion: 2,
      recordingIngestMaxBytes: RECORDING_INGEST_MAX_BYTES,
    },
  }),
  getTelemetry: getTelemetryState,
  updateTelemetry: setTelemetryConsent,
  listSessions: (query) => sessionQueryService.listSessions(query),
  getSession(sessionId) {
    const task = getTask(sessionId)
    return task ? sessionDetail(task) : null
  },
  getSessionState(sessionId) {
    // Liveness comes from the MCP identity service (a transport is
    // still attached); the audit store remembers ended sessions.
    if (identityService.getIdentity(sessionId)) return 'live'
    return getTask(sessionId) ? 'ended' : 'missing'
  },
  cancelSession: (sessionId) =>
    dispatchCancellation.cancelBySession(
      sessionId,
      'Cancelled through the BrowserClaw API',
    ),
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
  async appendRecordingEvents(identity, ndjson, batchId, hasGap) {
    const parsed = parseRecordingEvents(ndjson)
    const targetId =
      (await getTabTargetMap()?.targetForTab(identity.tabId)) ?? null
    if (parsed.events.length === 0) return { accepted: 0 }
    const appended = await recordingStore.appendBatch({
      documentId: identity.documentId,
      tabId: identity.tabId,
      targetId,
      events: parsed.events,
      batchId,
      hasGap: hasGap || parsed.droppedLines > 0,
    })
    return { accepted: appended ? parsed.events.length : 0 }
  },
  async getSessionBrowserTabPreview(sessionId, browserTabId) {
    const frame = await sessionQueryService.getSessionBrowserTabPreview(
      sessionId,
      browserTabId,
    )
    if (!frame) return null
    const bytes = Buffer.from(frame.jpegBase64, 'base64')
    return bytes.length === 0
      ? null
      : { bytes, etag: frame.capturedAt.toString() }
  },
  getDispatchScreenshot(dispatchId) {
    const path = screenshotPath(dispatchId)
    return existsSync(path)
      ? { bytes: readFileSync(path), etag: dispatchId.toString() }
      : null
  },
  async listConnections() {
    return { items: (await listBrowserosConnections()).map(connection) }
  },
  async connectHarness(harness) {
    return connection(await connectBrowserosToHarness(harness))
  },
  async disconnectHarness(harness) {
    return connection(await disconnectBrowserosFromHarness(harness))
  },
}

function knownSession(sessionId: string): boolean {
  return Boolean(identityService.getIdentity(sessionId) ?? getTask(sessionId))
}

function sessionDetail(task: TaskDetail): SessionDetail {
  const screenshotIds = new Set(task.screenshotDispatchIds)
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
        hasScreenshot: screenshotIds.has(row.id),
      }
    }),
  }
}

function connection(state: ConnectionState): Connection {
  return {
    harness: state.harness as Harness,
    installed: state.installed,
    ...(state.configPath === undefined ? {} : { configPath: state.configPath }),
    message: state.message,
  }
}

/** Tolerant parse of recorder-supplied NDJSON: lines that aren't JSON or lack a finite `ts` are dropped, never fatal. */
function parseRecordingEvents(ndjson: string): {
  events: RecordingEventInput[]
  droppedLines: number
} {
  const events: RecordingEventInput[] = []
  let droppedLines = 0
  for (const line of ndjson.split('\n')) {
    if (!line.trim()) continue
    try {
      const event = JSON.parse(line) as Record<string, unknown>
      if (typeof event.ts !== 'number' || !Number.isFinite(event.ts)) {
        droppedLines++
        continue
      }
      events.push({ ts: event.ts, type: event.type, data: event.data })
    } catch {
      droppedLines++
    }
  }
  return { events, droppedLines }
}
