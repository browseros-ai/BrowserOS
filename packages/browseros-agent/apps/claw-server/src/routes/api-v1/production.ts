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
import {
  CLAW_API_PORT_DEFAULT,
  type Connection,
  type Dispatch,
  type Harness,
  RECORDING_INGEST_MAX_BYTES,
  type SessionDetail,
  type SessionSummary,
  type Tab,
} from '@browseros/claw-api'
import { identityService } from '../../lib/mcp-session'
import {
  type TabActivityRecord,
  tabActivityRegistry,
} from '../../lib/tab-activity'
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
  getTask,
  listTasks,
  type TaskDetail,
  type TaskSummary,
} from '../../services/tasks'
import { VERSION } from '../../version'
import { resolveAgentDisplay } from '../tabs/agent-display'
import type { CanonicalApiDependencies, RecordingAssociation } from '.'

export const canonicalApiDependencies: CanonicalApiDependencies = {
  getSystemInfo: () => ({
    product: 'BrowserClaw',
    version: VERSION,
    url:
      getLocalServerUrl() ??
      `http://127.0.0.1:${CLAW_API_PORT_DEFAULT.toString()}`,
    capabilities: {
      recordingIngestMaxBytes: RECORDING_INGEST_MAX_BYTES,
    },
  }),
  getTelemetry: getTelemetryState,
  updateTelemetry: setTelemetryConsent,
  listSessions(query) {
    const result = listTasks({
      ...(query.slug ? { slug: query.slug } : {}),
      ...(query.status ? { status: query.status } : {}),
      ...(query.site ? { site: query.site } : {}),
      ...(query.search ? { search: query.search } : {}),
      ...(query.since !== undefined ? { since: query.since } : {}),
      ...(query.cursor !== undefined ? { cursor: query.cursor } : {}),
      ...(query.limit !== undefined ? { limit: query.limit } : {}),
    })
    // Browser profiles are a BrowserOS-native concept: only the Rust
    // server mints `profileId`, this server never does, so a profileId
    // filter here matches nothing. The filter exists for parity with
    // the contract's query surface.
    const items = result.tasks.map(sessionSummary)
    const filtered = query.profileId
      ? items.filter((item) => item.profileId === query.profileId)
      : items
    return {
      items: filtered,
      ...(result.nextCursor === null ? {} : { nextCursor: result.nextCursor }),
    }
  },
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
    // pageIds = tabs still claimed by this session whose targets have
    // recorded events — the per-tab views a replay can offer.
    const targetIds = new Set(metadata.targets.map((target) => target.targetId))
    const pageIds = tabActivityRegistry
      .snapshot()
      .filter(
        (tab) => tab.sessionId === sessionId && targetIds.has(tab.targetId),
      )
      .map((tab) => tab.pageId)
    return {
      hasData: metadata.exists,
      sizeBytes: metadata.sizeBytes,
      ...(metadata.firstEventAt === undefined
        ? {}
        : { firstEventAt: metadata.firstEventAt }),
      ...(metadata.lastEventAt === undefined
        ? {}
        : { lastEventAt: metadata.lastEventAt }),
      pageIds: Array.from(new Set(pageIds)).sort((a, b) => a - b),
    }
  },
  async downloadRecordingEvents(sessionId) {
    if (!knownSession(sessionId)) return null
    const events = await replayService.readSession(sessionId)
    return events.length === 0
      ? ''
      : `${events.map((event) => JSON.stringify(event)).join('\n')}\n`
  },
  async appendRecordingEvents(sessionId, association, ndjson, batchId) {
    const events = parseRecordingEvents(ndjson)
    const target = recordingTargetFor(
      tabActivityRegistry.snapshot(),
      sessionId,
      association,
    )
    if (!target) return null
    if (events.length === 0) return { accepted: 0 }
    const appended = await recordingStore.appendBatch(
      target.targetId,
      target.tabId,
      events,
      batchId,
    )
    return { accepted: appended ? events.length : 0 }
  },
  listTabs() {
    const identities = identityService.list()
    const identitiesByAgentId = new Map(
      identities.map((identity) => [identity.key, identity]),
    )
    return {
      items: tabActivityRegistry.snapshot().map((activity): Tab => {
        const display = resolveAgentDisplay(
          activity.agentId,
          activity.slug,
          identitiesByAgentId,
        )
        const preview = screencastCache.get(activity.pageId)
        return {
          tabId: activity.tabId,
          pageId: activity.pageId,
          targetId: activity.targetId,
          ...(identityService.getIdentity(activity.sessionId)
            ? { sessionId: activity.sessionId }
            : {}),
          slug: activity.slug,
          label: display.agentLabel,
          ...(display.harness === null ? {} : { harness: display.harness }),
          ...(display.color === null ? {} : { color: display.color }),
          url: activity.url,
          title: activity.title,
          status: activity.status,
          firstActivityAt: activity.firstToolAt,
          lastActivityAt: activity.lastToolAt,
          lastToolName: activity.lastToolName,
          toolCount: activity.toolCount,
          recentTools: activity.recentTools,
          ...(preview ? { previewCapturedAt: preview.capturedAt } : {}),
        }
      }),
    }
  },
  getTabPreview(pageId) {
    const frame = screencastCache.get(pageId)
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

/**
 * A batch lands only while the recorder's (tab, page, target) claim
 * still matches the live registry for that session. Any drift — the tab
 * reclaimed by another session, a navigation that swapped the target —
 * makes the batch undeliverable rather than attributing its events to
 * the wrong replay. Exported for the route tests.
 */
export function recordingTargetFor(
  tabs: TabActivityRecord[],
  sessionId: string,
  association: RecordingAssociation,
): TabActivityRecord | undefined {
  return tabs.find(
    (tab) =>
      tab.sessionId === sessionId &&
      tab.tabId === association.tabId &&
      tab.pageId === association.pageId &&
      tab.targetId === association.targetId,
  )
}

function knownSession(sessionId: string): boolean {
  return Boolean(identityService.getIdentity(sessionId) ?? getTask(sessionId))
}

function sessionSummary(task: TaskSummary): SessionSummary {
  return {
    sessionId: task.sessionId,
    slug: task.slug,
    label: task.agentLabel,
    name: identityService.getIdentity(task.sessionId)?.label ?? task.title,
    ...(task.site === null ? {} : { site: task.site }),
    startedAt: task.startedAt,
    ...(task.endedAt === null ? {} : { endedAt: task.endedAt }),
    durationMs: Math.max(0, task.durationMs),
    dispatchCount: task.dispatchCount,
    toolSequence: task.toolSequence,
    status: task.status,
    errorCount: task.errorCount,
    ...(task.lastScreenshotDispatchId === null
      ? {}
      : { lastScreenshotDispatchId: task.lastScreenshotDispatchId }),
  }
}

function sessionDetail(task: TaskDetail): SessionDetail {
  const screenshotIds = new Set(task.screenshotDispatchIds)
  return {
    session: sessionSummary(task),
    dispatches: task.dispatches.map((row): Dispatch => {
      return {
        dispatchId: row.id,
        createdAt: row.createdAt,
        slug: row.slug,
        label: row.agentLabel,
        sessionId: row.sessionId,
        toolName: row.toolName,
        ...(row.pageId === null ? {} : { pageId: row.pageId }),
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
function parseRecordingEvents(ndjson: string): RecordingEventInput[] {
  const events: RecordingEventInput[] = []
  for (const line of ndjson.split('\n')) {
    if (!line.trim()) continue
    try {
      const event = JSON.parse(line) as Record<string, unknown>
      if (typeof event.ts !== 'number' || !Number.isFinite(event.ts)) continue
      events.push({ ts: event.ts, type: event.type, data: event.data })
    } catch {}
  }
  return events
}
