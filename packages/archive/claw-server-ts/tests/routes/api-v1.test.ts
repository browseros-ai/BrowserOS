/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  mock,
  spyOn,
} from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import type {
  Connection,
  ConnectionList,
  RecordingMetadata,
  SessionDetail,
  SessionList,
  SessionSummary,
  SystemInfo,
  TelemetryState,
} from '@browseros/claw-api'
import { RECORDING_INGEST_MAX_BYTES } from '@browseros/shared/constants/limits'
import { setBrowserSession } from '../../src/lib/browser-session'
import { identityService } from '../../src/lib/mcp-session'
import {
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../src/modules/db/db'
import {
  type CanonicalApiDependencies,
  createCanonicalApiRoute,
} from '../../src/routes/api-v1'
import { createServer } from '../../src/server'
import { recordToolDispatch } from '../../src/services/audit-log'

const system: SystemInfo = {
  product: 'BrowserClaw',
  version: '1.2.3',
  url: 'http://127.0.0.1:9200',
  capabilities: {
    recordingIngestVersion: 2,
    recordingIngestMaxBytes: RECORDING_INGEST_MAX_BYTES,
  },
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
      screenshotId: 1,
    },
  ],
}
const recording: RecordingMetadata = {
  hasData: false,
  complete: true,
  sizeBytes: 0,
  tabs: [],
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
    getSessionPreview: async () => ({
      bytes: new Uint8Array([0xff, 0xd8]),
    }),
    listSessionScreenshots: () => ({
      items: [{ screenshotId: 1, capturedAt: 100, toolName: 'snapshot' }],
    }),
    getSessionScreenshot: () => ({
      bytes: new Uint8Array([0xff, 0xd8]),
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
  it('serves system, telemetry, session, and connection JSON envelopes', async () => {
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

  it('serves session NDJSON and accepts session-neutral document writes', async () => {
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

    const upload = await request(app, '/api/v1/recordings/events', {
      method: 'POST',
      headers: {
        'content-type': 'application/x-ndjson',
        'x-recording-tab-id': '101',
        'x-recording-document-id': '33D25F3CF060E81B14070BC356FF1871',
        'x-recording-batch-id': 'batch-1',
      },
      body: '{"ts":1}\n{"ts":2}\n',
    })
    expect(upload.status).toBe(200)
    expect(await upload.json()).toEqual({ accepted: 2 })
    expect(append).toHaveBeenCalledWith(
      {
        tabId: 101,
        documentId: '33D25F3CF060E81B14070BC356FF1871',
      },
      '{"ts":1}\n{"ts":2}\n',
      'batch-1',
      false,
    )

    const ended = createCanonicalApiRoute(
      dependencies({ getSessionState: () => 'ended' }),
    )
    const stillAccepted = await request(ended, '/api/v1/recordings/events', {
      method: 'POST',
      headers: {
        'content-type': 'application/x-ndjson',
        'x-recording-tab-id': '101',
        'x-recording-document-id': '8395FF2EF4A1D8579F1917B3B54ADECE',
        'x-recording-batch-id': 'batch-ended',
      },
      body: '{"ts":3}\n',
    })
    expect(stillAccepted.status).toBe(200)
  })

  it('rejects malformed Chrome recording document IDs', async () => {
    const append = mock(async () => ({ accepted: 1 }))
    const app = createCanonicalApiRoute(
      dependencies({ appendRecordingEvents: append }),
    )

    for (const documentId of [
      '33D25F3CF060E81B14070BC356FF187',
      '33D25F3CF060E81B14070BC356FF187Z',
      '018f47a7-1c2b-7def-8123-0123456789ab',
    ]) {
      const response = await request(app, '/api/v1/recordings/events', {
        method: 'POST',
        headers: {
          'content-type': 'application/x-ndjson',
          'x-recording-tab-id': '101',
          'x-recording-document-id': documentId,
          'x-recording-batch-id': 'malformed-document',
        },
        body: '{"ts":1}\n',
      })
      expect(response.status).toBe(400)
      expect(await response.json()).toMatchObject({ code: 'invalid_request' })
    }
    expect(append).not.toHaveBeenCalled()
  })

  it('rejects recording ingest from web-page origins before preflight', async () => {
    const app = createServer({ canonicalApiDependencies: dependencies() })
    const preflight = await app.request(
      'http://127.0.0.1/api/v1/recordings/events',
      {
        method: 'OPTIONS',
        headers: {
          origin: 'https://attacker.example',
          'access-control-request-method': 'POST',
          'access-control-request-headers':
            'content-type,x-recording-tab-id,x-recording-document-id,x-recording-batch-id',
        },
      },
    )
    expect(preflight.status).toBe(403)
    expect(preflight.headers.get('access-control-allow-origin')).toBeNull()

    const upload = await app.request(
      'http://127.0.0.1/api/v1/recordings/events',
      {
        method: 'POST',
        headers: {
          origin: 'https://attacker.example',
          'content-type': 'application/x-ndjson',
          'x-recording-tab-id': '101',
          'x-recording-document-id': '33D25F3CF060E81B14070BC356FF1871',
          'x-recording-batch-id': 'hostile-batch',
        },
        body: '{"ts":1}\n',
      },
    )
    expect(upload.status).toBe(403)
    expect(await upload.json()).toMatchObject({ code: 'forbidden' })
  })

  it('does not support session-scoped recording writes', async () => {
    const append = mock(async () => ({ accepted: 1 }))
    const app = createCanonicalApiRoute(
      dependencies({ appendRecordingEvents: append }),
    )

    const response = await request(
      app,
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
    expect(response.status).toBe(404)
    expect(append).not.toHaveBeenCalled()
  })

  it('enforces the recording byte ceiling before append', async () => {
    const append = mock(async () => ({ accepted: 1 }))
    const app = createCanonicalApiRoute(
      dependencies({ appendRecordingEvents: append }),
    )
    const headers = {
      'content-type': 'application/x-ndjson',
      'x-recording-tab-id': '101',
      'x-recording-document-id': '33D25F3CF060E81B14070BC356FF1871',
      'x-recording-batch-id': 'batch-boundary',
    }

    const accepted = await request(app, '/api/v1/recordings/events', {
      method: 'POST',
      headers,
      body: recordingLineOfBytes(RECORDING_INGEST_MAX_BYTES, 1),
    })
    expect(accepted.status).toBe(200)

    const rejected = await request(app, '/api/v1/recordings/events', {
      method: 'POST',
      headers,
      body: recordingLineOfBytes(RECORDING_INGEST_MAX_BYTES + 1, 2),
    })
    expect(rejected.status).toBe(413)
    expect(await rejected.json()).toMatchObject({
      code: 'recording_payload_too_large',
    })
    expect(append).toHaveBeenCalledTimes(1)
  })

  it('serves fresh previews and session-owned screenshot history', async () => {
    const app = createCanonicalApiRoute(dependencies())
    const preview = await request(app, '/api/v1/sessions/session-live/preview')
    expect(preview.status).toBe(200)
    expect(preview.headers.get('content-type')).toBe('image/jpeg')
    expect(preview.headers.get('cache-control')).toBe('private, no-store')

    const list = await request(app, '/api/v1/sessions/session-live/screenshots')
    expect(list.status).toBe(200)
    expect(await list.json()).toEqual({
      items: [{ screenshotId: 1, capturedAt: 100, toolName: 'snapshot' }],
    })

    const screenshot = await request(
      app,
      '/api/v1/sessions/session-live/screenshots/1',
    )
    expect(screenshot.status).toBe(200)
    expect(screenshot.headers.get('cache-control')).toContain('immutable')
  })

  it('validates screenshot ids before resolving bytes', async () => {
    const getScreenshot = mock(async () => null)
    const app = createCanonicalApiRoute(
      dependencies({ getSessionScreenshot: getScreenshot }),
    )

    for (const screenshotId of ['0', '-1', '1.5', 'nope']) {
      const response = await request(
        app,
        `/api/v1/sessions/session-live/screenshots/${screenshotId}`,
      )
      expect(response.status).toBe(400)
      expect(await response.json()).toMatchObject({ code: 'invalid_request' })
    }
    expect(getScreenshot).not.toHaveBeenCalled()
  })

  it('collapses every unresolved preview lookup to the same 404', async () => {
    const app = createCanonicalApiRoute(
      dependencies({ getSessionPreview: async () => null }),
    )
    const bodies = []
    for (const path of ['/api/v1/sessions/missing/preview']) {
      const response = await request(app, path)
      expect(response.status).toBe(404)
      bodies.push(await response.json())
    }
    expect(bodies).toEqual([
      {
        code: 'preview_not_found',
        message: 'session preview not found',
      },
    ])
  })

  it('returns canonical errors for missing resources and unknown harnesses', async () => {
    const app = createCanonicalApiRoute(
      dependencies({
        getSession: () => null,
        getRecording: () => null,
        getSessionPreview: async () => null,
        listSessionScreenshots: () => null,
        getSessionScreenshot: () => null,
      }),
    )
    const cases: Array<[string, number]> = [
      ['/api/v1/sessions/missing', 404],
      ['/api/v1/sessions/missing/recording', 404],
      ['/api/v1/sessions/missing/preview', 404],
      ['/api/v1/sessions/missing/screenshots', 404],
      ['/api/v1/sessions/missing/screenshots/1', 404],
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
    setBrowserSession(null)
    resetAuditDbForTesting()
  })

  it('keeps connected sessions visible when production page reconciliation fails', async () => {
    identityService.registerInitialize({
      sessionId: 'session-connected',
      clientInfo: { name: 'codex', version: '1.0.0', title: 'Codex CLI' },
    })
    setBrowserSession({
      pages: {
        list: async () => {
          throw new Error('CDP unavailable')
        },
      },
    } as unknown as BrowserSession)
    const consoleError = spyOn(console, 'error').mockImplementation(() => {})
    try {
      const response = await createServer().request(
        'http://localhost/api/v1/sessions?status=live',
      )
      expect(response.status).toBe(200)
      expect(await response.json()).toMatchObject({
        items: [
          {
            sessionId: 'session-connected',
            status: 'live',
            live: { state: 'idle', browserTabs: [] },
          },
        ],
      })
      expect(
        consoleError.mock.calls.some(([line]) =>
          String(line).includes(
            'live session browser reconciliation unavailable',
          ),
        ),
      ).toBe(true)
    } finally {
      consoleError.mockRestore()
    }
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
    })
    expect(body.dispatches[0]).not.toHaveProperty('screenshotId')
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
