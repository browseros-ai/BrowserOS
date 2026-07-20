/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import {
  type Connection,
  type ConnectionList,
  RECORDING_INGEST_MAX_BYTES,
  type RecordingMetadata,
  type SessionDetail,
  type SessionList,
  type SessionSummary,
  type SystemInfo,
  type TabList,
  type TelemetryState,
} from '@browseros/claw-api'
import { identityService } from '../../src/lib/mcp-session'
import {
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../src/modules/db/db'
import {
  type CanonicalApiDependencies,
  createCanonicalApiRoute,
} from '../../src/routes/api-v1'
import { recordingTargetFor } from '../../src/routes/api-v1/production'
import { createServer } from '../../src/server'
import { recordToolDispatch } from '../../src/services/audit-log'

const system: SystemInfo = {
  product: 'BrowserClaw',
  version: '1.2.3',
  url: 'http://127.0.0.1:9200',
  capabilities: { recordingIngestMaxBytes: RECORDING_INGEST_MAX_BYTES },
}
const telemetry: TelemetryState = {
  distinctId: 'install-1',
  enabled: true,
  consent: true,
}
const liveSession: SessionSummary = {
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
const sessions: SessionList = { items: [liveSession] }
const sessionDetail: SessionDetail = {
  session: liveSession,
  dispatches: [
    {
      dispatchId: 1,
      createdAt: 100,
      slug: 'codex',
      label: 'Codex',
      sessionId: 'session-live',
      toolName: 'snapshot',
      hasScreenshot: true,
    },
  ],
}
const recording: RecordingMetadata = {
  hasData: false,
  sizeBytes: 0,
  pageIds: [],
}
const tabs: TabList = {
  items: [
    {
      tabId: 101,
      pageId: 7,
      targetId: 'target-7',
      sessionId: 'session-live',
      slug: 'codex',
      label: 'Codex',
      url: 'https://browseros.com',
      title: 'BrowserOS',
      status: 'active',
      firstActivityAt: 100,
      lastActivityAt: 110,
      lastToolName: 'snapshot',
      toolCount: 1,
      recentTools: [{ name: 'snapshot', at: 110 }],
      previewCapturedAt: 111,
    },
  ],
}
const connection: Connection = {
  harness: 'Codex',
  installed: true,
  message: 'Configured in Codex.',
}
const connections: ConnectionList = { items: [connection] }

function dependencies(
  overrides: Partial<CanonicalApiDependencies> = {},
): CanonicalApiDependencies {
  return {
    getSystemInfo: () => system,
    getTelemetry: () => telemetry,
    updateTelemetry: () => telemetry,
    listSessions: () => sessions,
    getSession: () => sessionDetail,
    getSessionState: () => 'live',
    cancelSession: () => 0,
    getRecording: () => recording,
    downloadRecordingEvents: async () => '',
    appendRecordingEvents: async () => ({ accepted: 2 }),
    listTabs: () => tabs,
    getTabPreview: () => ({ bytes: new Uint8Array([0xff, 0xd8]), etag: '111' }),
    getDispatchScreenshot: () => ({
      bytes: new Uint8Array([0xff, 0xd8]),
      etag: '1',
    }),
    listConnections: async () => connections,
    connectHarness: async () => connection,
    disconnectHarness: async () => ({ ...connection, installed: false }),
    ...overrides,
  }
}

function request(
  app: ReturnType<typeof createCanonicalApiRoute>,
  path: string,
  init?: RequestInit,
) {
  return app.request(`http://localhost${path}`, init)
}

function recordingLineOfBytes(bytes: number, timestamp: number): string {
  const prefix = `{"ts":${timestamp.toString()},"type":2,"data":{"html":"`
  const suffix = '"}}'
  return `${prefix}${'x'.repeat(bytes - prefix.length - suffix.length)}${suffix}`
}

describe('canonical TypeScript API', () => {
  it('serves system, telemetry, session, tab, and connection JSON envelopes', async () => {
    const app = createCanonicalApiRoute(dependencies())
    const cases: Array<[string, string, unknown, RequestInit | undefined]> = [
      ['/api/v1/system', 'GET', system, undefined],
      ['/api/v1/settings/telemetry', 'GET', telemetry, undefined],
      [
        '/api/v1/settings/telemetry',
        'PUT',
        telemetry,
        {
          method: 'PUT',
          body: JSON.stringify({ consent: true }),
          headers: { 'content-type': 'application/json' },
        },
      ],
      ['/api/v1/sessions', 'GET', sessions, undefined],
      ['/api/v1/sessions/session-live', 'GET', sessionDetail, undefined],
      ['/api/v1/sessions/session-live/recording', 'GET', recording, undefined],
      ['/api/v1/tabs', 'GET', tabs, undefined],
      ['/api/v1/connections', 'GET', connections, undefined],
      ['/api/v1/connections/Codex', 'PUT', connection, { method: 'PUT' }],
      [
        '/api/v1/connections/Codex',
        'DELETE',
        { ...connection, installed: false },
        { method: 'DELETE' },
      ],
    ]

    for (const [path, method, expected, init] of cases) {
      const response = await request(app, path, init ?? { method })
      expect(response.status, `${method} ${path}`).toBe(200)
      expect(await response.json(), `${method} ${path}`).toEqual(expected)
    }
  })

  it('validates list and telemetry input with canonical errors', async () => {
    const app = createCanonicalApiRoute(dependencies())
    const badLimit = await request(app, '/api/v1/sessions?limit=0')
    expect(badLimit.status).toBe(400)
    expect(await badLimit.json()).toMatchObject({
      code: 'invalid_request',
      message: expect.any(String),
    })

    const badTelemetry = await request(app, '/api/v1/settings/telemetry', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ consent: 'yes' }),
    })
    expect(badTelemetry.status).toBe(400)
    expect(await badTelemetry.json()).toMatchObject({ code: 'invalid_request' })
  })

  it('distinguishes missing, ended, and idle live session cancellation', async () => {
    for (const [state, status, code] of [
      ['missing', 404, 'session_not_found'],
      ['ended', 409, 'session_not_live'],
    ] as const) {
      const app = createCanonicalApiRoute(
        dependencies({ getSessionState: () => state }),
      )
      const response = await request(app, '/api/v1/sessions/session-1/cancel', {
        method: 'POST',
      })
      expect(response.status).toBe(status)
      expect(await response.json()).toMatchObject({ code })
    }

    const app = createCanonicalApiRoute(
      dependencies({ cancelSession: () => 0 }),
    )
    const response = await request(
      app,
      '/api/v1/sessions/session-live/cancel',
      {
        method: 'POST',
      },
    )
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ cancelled: 0 })
  })

  it('serves NDJSON and rejects writes after a session ends', async () => {
    const append = mock(async () => ({ accepted: 2 }))
    const app = createCanonicalApiRoute(
      dependencies({
        downloadRecordingEvents: async () => '{"type":2}\n',
        appendRecordingEvents: append,
      }),
    )
    const download = await request(
      app,
      '/api/v1/sessions/session-live/recording/events',
    )
    expect(download.status).toBe(200)
    expect(download.headers.get('content-type')).toContain(
      'application/x-ndjson',
    )
    expect(await download.text()).toBe('{"type":2}\n')

    const upload = await request(
      app,
      '/api/v1/sessions/session-live/recording/events',
      {
        method: 'POST',
        headers: {
          'content-type': 'application/x-ndjson',
          'x-recording-tab-id': '101',
          'x-recording-page-id': '7',
          'x-recording-target-id': 'target-7',
          'x-recording-batch-id': 'batch-1',
        },
        body: '{"ts":1}\n{"ts":2}\n',
      },
    )
    expect(upload.status).toBe(200)
    expect(await upload.json()).toEqual({ accepted: 2 })
    expect(append).toHaveBeenCalledWith(
      'session-live',
      { tabId: 101, pageId: 7, targetId: 'target-7' },
      '{"ts":1}\n{"ts":2}\n',
      'batch-1',
    )

    const changed = createCanonicalApiRoute(
      dependencies({ appendRecordingEvents: async () => null }),
    )
    const changedResponse = await request(
      changed,
      '/api/v1/sessions/session-live/recording/events',
      {
        method: 'POST',
        headers: {
          'content-type': 'application/x-ndjson',
          'x-recording-tab-id': '101',
          'x-recording-page-id': '7',
          'x-recording-target-id': 'stale-target',
        },
        body: '{"ts":1}\n',
      },
    )
    expect(changedResponse.status).toBe(409)
    expect(await changedResponse.json()).toMatchObject({
      code: 'recording_association_changed',
    })

    const ended = createCanonicalApiRoute(
      dependencies({ getSessionState: () => 'ended' }),
    )
    const rejected = await request(
      ended,
      '/api/v1/sessions/session-old/recording/events',
      { method: 'POST', body: '{"ts":1}\n' },
    )
    expect(rejected.status).toBe(410)
    expect(await rejected.json()).toMatchObject({ code: 'session_ended' })
  })

  it('enforces the recording byte ceiling before append', async () => {
    const append = mock(async () => ({ accepted: 1 }))
    const app = createCanonicalApiRoute(
      dependencies({ appendRecordingEvents: append }),
    )
    const headers = {
      'content-type': 'application/x-ndjson',
      'x-recording-tab-id': '101',
      'x-recording-page-id': '7',
      'x-recording-target-id': 'target-7',
    }

    const accepted = await request(
      app,
      '/api/v1/sessions/session-live/recording/events',
      {
        method: 'POST',
        headers,
        body: recordingLineOfBytes(RECORDING_INGEST_MAX_BYTES, 1),
      },
    )
    expect(accepted.status).toBe(200)

    const rejected = await request(
      app,
      '/api/v1/sessions/session-live/recording/events',
      {
        method: 'POST',
        headers,
        body: recordingLineOfBytes(RECORDING_INGEST_MAX_BYTES + 1, 2),
      },
    )
    expect(rejected.status).toBe(413)
    expect(await rejected.json()).toMatchObject({
      code: 'recording_payload_too_large',
    })
    expect(append).toHaveBeenCalledTimes(1)
  })

  it('selects the exact recording association in a multi-tab session', () => {
    const first = {
      targetId: 'target-7',
      tabId: 101,
      pageId: 7,
      sessionId: 'session-live',
      agentId: 'codex-one',
      slug: 'codex',
      url: 'https://one.example',
      title: 'One',
      firstToolAt: 1,
      lastToolAt: 2,
      lastToolName: 'snapshot',
      toolCount: 1,
      recentTools: [],
      status: 'active' as const,
    }
    const second = {
      ...first,
      targetId: 'target-8',
      tabId: 102,
      pageId: 8,
      url: 'https://two.example',
      title: 'Two',
    }

    expect(
      recordingTargetFor([first, second], 'session-live', {
        tabId: 102,
        pageId: 8,
        targetId: 'target-8',
      }),
    ).toBe(second)
    expect(
      recordingTargetFor([first, second], 'session-live', {
        tabId: 102,
        pageId: 8,
        targetId: 'target-7',
      }),
    ).toBeUndefined()
  })

  it('serves binary artifacts without embedding preview bytes in tab JSON', async () => {
    const app = createCanonicalApiRoute(dependencies())
    const preview = await request(app, '/api/v1/tabs/7/preview')
    expect(preview.status).toBe(200)
    expect(preview.headers.get('content-type')).toBe('image/jpeg')
    expect(preview.headers.get('cache-control')).toBe(
      'private, max-age=0, must-revalidate',
    )

    const screenshot = await request(app, '/api/v1/dispatches/1/screenshot')
    expect(screenshot.status).toBe(200)
    expect(screenshot.headers.get('cache-control')).toContain('immutable')
    expect(JSON.stringify(tabs)).not.toContain('jpegBase64')
  })

  it('returns canonical errors for missing resources and unknown harnesses', async () => {
    const app = createCanonicalApiRoute(
      dependencies({
        getSession: () => null,
        getRecording: () => null,
        getTabPreview: () => null,
        getDispatchScreenshot: () => null,
      }),
    )
    const cases: Array<[string, number]> = [
      ['/api/v1/sessions/missing', 404],
      ['/api/v1/sessions/missing/recording', 404],
      ['/api/v1/tabs/7/preview', 404],
      ['/api/v1/dispatches/1/screenshot', 404],
      ['/api/v1/connections/Unknown', 404],
    ]
    for (const [path, status] of cases) {
      const method = path.includes('/connections/') ? 'PUT' : 'GET'
      const response = await request(app, path, { method })
      expect(response.status, path).toBe(status)
      expect(await response.json(), path).toMatchObject({
        code: expect.any(String),
        message: expect.any(String),
      })
    }
  })

  it('uses the canonical error shape for unexpected mounted failures', async () => {
    const server = createServer({
      canonicalApiDependencies: dependencies({
        listSessions: () => {
          throw new Error('database unavailable')
        },
      }),
    })
    const response = await server.request('http://localhost/api/v1/sessions')
    expect(response.status).toBe(500)
    expect(await response.json()).toMatchObject({
      code: 'internal_error',
      message: 'internal server error',
      requestId: expect.any(String),
    })
  })
})

describe('mounted canonical TypeScript API', () => {
  beforeEach(() => {
    setAuditDbForTesting()
    identityService.clear()
  })

  afterEach(() => {
    identityService.clear()
    resetAuditDbForTesting()
  })

  it('maps persisted task rows without leaking internal identities or nulls', async () => {
    recordToolDispatch({
      agentId: 'codex-generated-name',
      slug: 'codex',
      agentLabel: 'Codex',
      sessionId: 'session-1',
      toolName: 'snapshot',
      pageId: 7,
      targetId: 'target-7',
      url: null,
      title: null,
      rawArgs: {},
      durationMs: 5,
      result: { isError: false, content: [], structuredContent: {} },
    })

    const response = await createServer().request(
      'http://localhost/api/v1/sessions/session-1',
    )
    expect(response.status).toBe(200)
    const body = (await response.json()) as SessionDetail
    expect(body.session).toMatchObject({
      sessionId: 'session-1',
      slug: 'codex',
      label: 'Codex',
      status: 'live',
    })
    expect(body.dispatches[0]).toMatchObject({
      dispatchId: 1,
      pageId: 7,
      targetId: 'target-7',
      hasScreenshot: false,
    })
    expect(JSON.stringify(body)).not.toContain('agentId')
    expect(JSON.stringify(body)).not.toContain(':null')
  })

  it('copies the mounted request id into canonical errors', async () => {
    const response = await createServer().request(
      'http://localhost/api/v1/sessions/missing',
    )
    const body = (await response.json()) as { requestId?: string }
    expect(response.status).toBe(404)
    expect(body.requestId).toBeString()
    expect(response.headers.get('x-request-id')).toBe(body.requestId)
  })
})
