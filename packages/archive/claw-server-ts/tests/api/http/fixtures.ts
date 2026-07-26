import type {
  Connection,
  RecordingMetadata,
  SessionDetail,
  SessionList,
  SessionSummary,
  SystemInfo,
  TelemetryState,
} from '@browseros/claw-api'
import { RECORDING_INGEST_MAX_BYTES } from '@browseros/shared/constants/limits'
import { createHttpRoute, type HttpHandlerSet } from '../../../src/api/http'
import { createConnectionHandlers } from '../../../src/api/http/handlers/connections'
import { createPreviewHandlers } from '../../../src/api/http/handlers/previews'
import { createRecordingHandlers } from '../../../src/api/http/handlers/recordings'
import { createReplayHandlers } from '../../../src/api/http/handlers/replay'
import { createScreenshotHandlers } from '../../../src/api/http/handlers/screenshots'
import { createSessionHandlers } from '../../../src/api/http/handlers/sessions'
import { createSettingsHandlers } from '../../../src/api/http/handlers/settings'
import { createSystemHandlers } from '../../../src/api/http/handlers/system'

export const system: SystemInfo = {
  product: 'BrowserClaw',
  version: '1.2.3',
  url: 'http://127.0.0.1:9200',
  capabilities: {
    recordingIngestVersion: 2,
    recordingIngestMaxBytes: RECORDING_INGEST_MAX_BYTES,
  },
}

export const telemetry: TelemetryState = {
  distinctId: 'install-1',
  enabled: true,
  consent: true,
}

export const liveSession: SessionSummary = {
  sessionId: 'session-live',
  slug: 'codex',
  label: 'Codex',
  name: 'Research BrowserClaw',
  startedAt: 100,
  durationMs: 10,
  dispatchCount: 1,
  toolSequence: ['snapshot'],
  status: 'live',
  errorCount: 0,
}

export const sessions: SessionList = { items: [liveSession] }

export const sessionDetail: SessionDetail = {
  session: liveSession,
  dispatches: [
    {
      dispatchId: 1,
      createdAt: 100,
      slug: 'codex',
      label: 'Codex',
      sessionId: 'session-live',
      toolName: 'snapshot',
      screenshotId: 1,
    },
  ],
}

export const recording: RecordingMetadata = {
  hasData: false,
  complete: true,
  sizeBytes: 0,
  tabs: [],
}

export const connection: Connection = {
  harness: 'Codex',
  installed: true,
  message: 'Configured in Codex.',
}

export function defaultHttpHandlers(): HttpHandlerSet {
  return {
    system: createSystemHandlers({ getSystemInfo: () => system }),
    settings: createSettingsHandlers({
      getTelemetry: () => telemetry,
      updateTelemetry: () => telemetry,
    }),
    sessions: createSessionHandlers({
      listSessions: () => sessions,
      getSession: () => sessionDetail,
      getSessionState: () => 'live',
      cancelSession: () => 0,
    }),
    recordings: createRecordingHandlers({
      appendRecordingEvents: async (_identity, events) => ({
        accepted: events.length,
      }),
    }),
    replay: createReplayHandlers({
      getRecording: () => recording,
      downloadRecordingEvents: async () => '',
    }),
    previews: createPreviewHandlers({
      getSessionPreview: () => ({ bytes: new Uint8Array([0xff, 0xd8]) }),
    }),
    screenshots: createScreenshotHandlers({
      listSessionScreenshots: () => ({
        items: [{ screenshotId: 1, capturedAt: 100, toolName: 'snapshot' }],
      }),
      getSessionScreenshot: () => ({
        bytes: new Uint8Array([0xff, 0xd8]),
      }),
    }),
    connections: createConnectionHandlers({
      listConnections: async () => ({ items: [connection] }),
      connectHarness: async () => connection,
      disconnectHarness: async () => ({ ...connection, installed: false }),
    }),
  }
}

export function httpTestApp(overrides: Partial<HttpHandlerSet> = {}) {
  return createHttpRoute({ ...defaultHttpHandlers(), ...overrides })
}
