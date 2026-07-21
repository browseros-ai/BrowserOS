/**
 * The behavioral half of the contract: every case runs verbatim against
 * both server implementations through a test-local typed client (raw fetch
 * where the client can't express the check), asserting observable wire
 * behavior only — status codes, envelopes, headers — never anything
 * implementation-specific. A case that passes on one server and fails
 * on the other is by definition a contract violation.
 *
 * Cases assume the seeded world the adapters provide (two same-profile
 * live sessions, a zero-tab live session, an ended session, browser tab
 * 101, and two session screenshots). The `shutdown` case must stay last:
 * it kills the server it runs against.
 */

import { expect } from 'bun:test'
import {
  type ApiError,
  Harness,
  SessionStatus,
} from '../../../packages/claw-api/src'
import { RECORDING_INGEST_MAX_BYTES } from '../../../packages/shared/src/constants/limits'
import { ContractHttpError } from './http-client'
import type { ContractServer } from './server-adapters'

export interface ContractCase {
  name: string
  run(server: ContractServer): Promise<void>
}

// Legacy vocabulary the canonical surface must never leak: sessions
// replaced agents/tasks/runs. Binary frames travel through preview and
// screenshot endpoints rather than the live browser-tab projection.
const FORBIDDEN_IDENTITY_KEYS = ['agent', 'task', 'run'].map(
  (scope) => `${scope}Id`,
)
const INLINE_JPEG_KEY = ['jpeg', 'Base64'].join('')
const RETIRED_BROWSER_TAB_PREVIEW = [
  '/api/v1/sessions/session-1',
  'browser-tabs',
  '7',
  'preview',
].join('/')
const RETIRED_DISPATCH_SCREENSHOT = [
  '/api/v1',
  'dispatches',
  '1',
  'screenshot',
].join('/')
const RETIRED_ROUTES = [
  ['GET', '/system/version'],
  ['GET', '/system/url'],
  ['GET', '/system/telemetry'],
  ['POST', '/system/telemetry'],
  ['POST', '/agents/agent-1/cancel'],
  ['GET', '/tabs/activity'],
  ['GET', '/api/v1/tabs'],
  ['GET', '/api/v1/tabs/7/preview'],
  ['GET', '/connections'],
  ['POST', '/connections/NotAHarness/connect'],
  ['POST', '/connections/NotAHarness/disconnect'],
  ['GET', '/audit/dispatches'],
  ['GET', '/audit/tasks'],
  ['GET', '/audit/tasks/session-1'],
  ['GET', '/audit/screenshot/1'],
  ['GET', '/recordings/health'],
  ['POST', '/recordings/tabs/1/events'],
  ['GET', '/audit/replays/session-1'],
  ['GET', '/audit/replays/session-1/meta'],
  ['GET', RETIRED_BROWSER_TAB_PREVIEW],
  ['GET', RETIRED_DISPATCH_SCREENSHOT],
] as const

export const contractCases: ContractCase[] = [
  {
    name: 'health',
    async run({ api }) {
      expect(await api.getHealth()).toEqual({ status: 'ok' })
    },
  },
  {
    name: 'system info',
    async run({ api }) {
      const value = await api.getSystemInfo()
      expect(value.product).toBe('BrowserClaw')
      expect(value.version).toBeString()
      expect(value.url).toStartWith('http://127.0.0.1:')
      expect(value.capabilities?.recordingIngestMaxBytes).toBe(
        RECORDING_INGEST_MAX_BYTES,
      )
      expect(value.capabilities?.recordingIngestVersion).toBe(2)
    },
  },
  {
    name: 'telemetry read and update',
    async run({ api }) {
      expect((await api.getTelemetry()).distinctId).toBeString()
      const updated = await api.updateTelemetry({
        updateTelemetryRequest: { consent: false },
      })
      expect(updated.consent).toBe(false)
      expect((await api.getTelemetry()).consent).toBe(false)
    },
  },
  {
    name: 'session list and detail',
    async run({ api, liveSessionId }) {
      const list = await api.listSessions({ limit: 100 })
      expect(list.items.some((item) => item.sessionId === liveSessionId)).toBe(
        true,
      )
      const detail = await api.getSession({ sessionId: liveSessionId })
      expect(detail.session.sessionId).toBe(liveSessionId)
      expect(detail.session.latestScreenshotId).toBe(2)
      expect(detail.dispatches[0]).toMatchObject({
        dispatchId: 1,
        pageId: 7,
        tabId: 101,
        targetId: 'target-7',
        screenshotId: 1,
      })
      expect(detail.dispatches[1]?.screenshotId).toBe(2)
      const serialized = JSON.stringify(detail)
      for (const key of FORBIDDEN_IDENTITY_KEYS) {
        expect(serialized).not.toContain(key)
      }
      // Absent optional fields are omitted entirely, never null — the
      // reason both servers build responses field-conditionally.
      expect(JSON.stringify(detail)).not.toContain(':null')
    },
  },
  {
    name: 'idle live cancellation',
    async run({ api, liveSessionId }) {
      expect(await api.cancelSession({ sessionId: liveSessionId })).toEqual({
        cancelled: 0,
      })
    },
  },
  {
    name: 'empty recording metadata',
    async run({ api, liveSessionId }) {
      expect(await api.getRecording({ sessionId: liveSessionId })).toEqual({
        hasData: false,
        complete: true,
        sizeBytes: 0,
        tabs: [],
      })
    },
  },
  {
    name: 'recording append and NDJSON download',
    async run({ api, baseUrl, liveSessionId }) {
      const body =
        '{"type":2,"data":{},"ts":100}\n{"type":3,"data":{},"ts":200}\n'
      const request = {
        xRecordingTabId: 101,
        xRecordingDocumentId: '33D25F3CF060E81B14070BC356FF1871',
        xRecordingBatchId: 'contract-batch-1',
        body,
      }
      expect(await api.appendRecordingEvents(request)).toEqual({ accepted: 2 })
      expect(await api.appendRecordingEvents(request)).toEqual({ accepted: 0 })
      expect(
        await api.getRecording({ sessionId: liveSessionId }),
      ).toMatchObject({
        hasData: true,
        complete: true,
        tabs: [
          {
            tabId: 101,
            segments: [
              {
                documentId: '33D25F3CF060E81B14070BC356FF1871',
                hasGap: false,
              },
            ],
          },
        ],
      })
      const response = await fetch(
        `${baseUrl}/api/v1/sessions/${liveSessionId}/recording/events`,
      )
      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toContain(
        'application/x-ndjson',
      )
      expect((await response.text()).trim().split('\n')).toHaveLength(2)
    },
  },
  {
    name: 'session recording ingest is unsupported',
    async run({ baseUrl, liveSessionId }) {
      const path = `${baseUrl}/api/v1/sessions/${liveSessionId}/recording/events`
      const before = await fetch(path).then((response) => response.text())
      const response = await fetch(path, {
        method: 'POST',
        headers: {
          'content-type': 'application/x-ndjson',
          'x-recording-tab-id': '101',
          'x-recording-page-id': '7',
          'x-recording-target-id': 'target-7',
        },
        body: '{"ts":175,"data":{"id":"legacy-write"}}\n',
      })
      expect([404, 405]).toContain(response.status)
      const after = await fetch(path).then((download) => download.text())
      expect(after).toBe(before)
      expect(after).not.toContain('legacy-write')
    },
  },
  {
    name: 'recording ingest rejects malformed Chrome document IDs',
    async run({ api }) {
      for (const [index, documentId] of [
        '33D25F3CF060E81B14070BC356FF187',
        '33D25F3CF060E81B14070BC356FF187Z',
        '018f47a7-1c2b-7def-8123-0123456789ab',
      ].entries()) {
        await expectApiError(
          () =>
            api.appendRecordingEvents({
              xRecordingTabId: 101,
              xRecordingDocumentId: documentId,
              xRecordingBatchId: `contract-malformed-${index.toString()}`,
              body: '{"ts":125,"type":3,"data":{}}\n',
            }),
          400,
          'invalid_request',
        )
      }
    },
  },
  {
    name: 'recording ingest rejects web origins',
    async run({ baseUrl }) {
      const path = `${baseUrl}/api/v1/recordings/events`
      const preflight = await fetch(path, {
        method: 'OPTIONS',
        headers: {
          origin: 'https://attacker.example',
          'access-control-request-method': 'POST',
          'access-control-request-headers':
            'content-type,x-recording-tab-id,x-recording-document-id,x-recording-batch-id',
        },
      })
      expect(preflight.status).toBe(403)
      expect(preflight.headers.get('access-control-allow-origin')).toBeNull()

      const response = await fetch(path, {
        method: 'POST',
        headers: {
          origin: 'https://attacker.example',
          'content-type': 'application/x-ndjson',
          'x-recording-tab-id': '101',
          'x-recording-document-id': '33D25F3CF060E81B14070BC356FF1871',
          'x-recording-batch-id': 'hostile-contract-batch',
        },
        body: '{"ts":150,"type":3,"data":{}}\n',
      })
      expect(response.status).toBe(403)
      expect(await response.json()).toMatchObject({ code: 'forbidden' })

      const trusted = await fetch(path, {
        method: 'POST',
        headers: {
          origin: 'chrome-extension://pjimfkbpehlcllblajnpfamdfjhhlgkc',
          'content-type': 'application/x-ndjson',
          'x-recording-tab-id': '101',
          'x-recording-document-id': '8395FF2EF4A1D8579F1917B3B54ADECE',
          'x-recording-batch-id': 'trusted-contract-batch',
        },
        body: '{"ts":250,"type":3,"data":{}}\n',
      })
      expect(trusted.status).toBe(200)
      expect(await trusted.json()).toEqual({ accepted: 1 })
    },
  },
  {
    name: 'recording ingest byte ceiling',
    async run({ api }) {
      const accepted = recordingLineOfBytes(RECORDING_INGEST_MAX_BYTES, 400)
      expect(
        await api.appendRecordingEvents({
          xRecordingTabId: 101,
          xRecordingDocumentId: '9E84CDCAB8762569B5B109D125F60147',
          xRecordingBatchId: 'contract-boundary',
          body: accepted,
        }),
      ).toEqual({ accepted: 1 })

      await expectApiError(
        () =>
          api.appendRecordingEvents({
            xRecordingTabId: 101,
            xRecordingDocumentId: 'A18F47A71C2B7DEF81230123456789AC',
            xRecordingBatchId: 'contract-over-limit',
            body: recordingLineOfBytes(RECORDING_INGEST_MAX_BYTES + 1, 401),
          }),
        413,
        'recording_payload_too_large',
      )

      expect(
        await api.appendRecordingEvents({
          xRecordingTabId: 101,
          xRecordingDocumentId: 'B18F47A71C2B7DEF81230123456789AD',
          xRecordingBatchId: 'contract-after-over-limit',
          body: '{"ts":402,"type":3,"data":{}}',
        }),
      ).toEqual({ accepted: 1 })
    },
  },
  {
    name: 'live session projection and session preview',
    async run({
      api,
      baseUrl,
      liveSessionId,
      secondLiveSessionId,
      zeroTabLiveSessionId,
    }) {
      const snapshot = await api.listSessions({ status: SessionStatus.Live })
      expect(snapshot.nextCursor).toBeUndefined()

      const primary = snapshot.items.find(
        (session) => session.sessionId === liveSessionId,
      )
      const second = snapshot.items.find(
        (session) => session.sessionId === secondLiveSessionId,
      )
      const zeroTab = snapshot.items.find(
        (session) => session.sessionId === zeroTabLiveSessionId,
      )
      expect(primary).toMatchObject({
        profileId: 'profile-shared',
        harness: 'Codex',
        color: '#7A5AF8',
        live: {
          state: 'active',
          browserTabs: [
            {
              browserTabId: 101,
              url: 'https://browseros.com',
              title: 'BrowserOS',
              toolCount: 1,
              recentTools: [{ name: 'snapshot', at: 110 }],
            },
            {
              browserTabId: 102,
              url: 'https://example.com',
              title: 'Example Domain',
              toolCount: 0,
              recentTools: [],
            },
          ],
        },
      })
      expect(second).toMatchObject({
        profileId: primary?.profileId,
        live: { state: 'idle' },
      })
      expect(second?.sessionId).not.toBe(primary?.sessionId)
      expect(zeroTab?.live).toEqual({ state: 'idle', browserTabs: [] })

      const rawResponse = await fetch(`${baseUrl}/api/v1/sessions?status=live`)
      expect(rawResponse.status).toBe(200)
      const rawSnapshot = (await rawResponse.json()) as {
        items: Array<{
          sessionId: string
          live?: { browserTabs?: unknown }
        }>
      }
      const rawPrimary = rawSnapshot.items.find(
        (session) => session.sessionId === liveSessionId,
      )
      expect(rawPrimary).toBeDefined()
      expect(Array.isArray(rawPrimary?.live?.browserTabs)).toBe(true)
      const rawBrowserTabs = JSON.stringify(rawPrimary?.live?.browserTabs)
      for (const forbiddenKey of [
        'pageId',
        'targetId',
        'sessionId',
        'profileId',
        'slug',
        'label',
        'harness',
        'color',
        INLINE_JPEG_KEY,
        ['preview', 'CapturedAt'].join(''),
      ]) {
        expect(rawBrowserTabs).not.toContain(`"${forbiddenKey}"`)
      }

      for (const sessionId of [liveSessionId, secondLiveSessionId]) {
        const preview = await api.getSessionPreview({ sessionId })
        expect(preview.type).toBe('image/jpeg')
        expect(Array.from(new Uint8Array(await preview.arrayBuffer()))).toEqual(
          [0xff, 0xd8],
        )
      }

      const previewResponse = await fetch(
        `${baseUrl}/api/v1/sessions/${liveSessionId}/preview`,
      )
      expect(previewResponse.status).toBe(200)
      expect(previewResponse.headers.get('cache-control')).toBe(
        'private, no-store',
      )
    },
  },
  {
    name: 'ordered session screenshot history',
    async run({ api, baseUrl, liveSessionId, screenshotIds }) {
      const list = await api.listSessionScreenshots({
        sessionId: liveSessionId,
      })
      expect(list.items.map((item) => item.screenshotId)).toEqual(screenshotIds)
      expect(list.items.map((item) => item.capturedAt)).toEqual(
        [...list.items]
          .sort((left, right) =>
            left.capturedAt === right.capturedAt
              ? left.screenshotId - right.screenshotId
              : left.capturedAt - right.capturedAt,
          )
          .map((item) => item.capturedAt),
      )
      expect(list.items.every((item) => item.toolName === 'snapshot')).toBe(
        true,
      )

      for (const screenshotId of screenshotIds) {
        const screenshot = await api.getSessionScreenshot({
          sessionId: liveSessionId,
          screenshotId,
        })
        expect(screenshot.type).toBe('image/jpeg')
        expect(
          Array.from(new Uint8Array(await screenshot.arrayBuffer())),
        ).toEqual([0xff, 0xd8])
      }

      const response = await fetch(
        `${baseUrl}/api/v1/sessions/${liveSessionId}/screenshots/${screenshotIds[0].toString()}`,
      )
      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toBe('image/jpeg')
      expect(response.headers.get('cache-control')).toBe(
        'public, max-age=31536000, immutable',
      )
    },
  },
  {
    name: 'connection lifecycle',
    async run({ api }) {
      expect(Array.isArray((await api.listConnections()).items)).toBe(true)
      expect(
        await api.connectHarness({ harness: Harness.Codex }),
      ).toMatchObject({ harness: Harness.Codex, installed: true })
      expect(
        await api.disconnectHarness({ harness: Harness.Codex }),
      ).toMatchObject({ harness: Harness.Codex, installed: false })
    },
  },
  {
    name: 'invalid limit',
    async run({ api }) {
      await expectApiError(
        () => api.listSessions({ limit: 0 }),
        400,
        'invalid_request',
      )
    },
  },
  {
    name: 'invalid screenshot id',
    async run({ baseUrl, liveSessionId }) {
      const response = await fetch(
        `${baseUrl}/api/v1/sessions/${liveSessionId}/screenshots/0`,
      )
      expect(response.status).toBe(400)
      expect(await response.json()).toMatchObject({ code: 'invalid_request' })
    },
  },
  {
    name: 'unknown session',
    async run({ api }) {
      await expectApiError(
        () => api.getSession({ sessionId: 'missing' }),
        404,
        'session_not_found',
      )
    },
  },
  {
    name: 'recording write is session neutral',
    async run({ api }) {
      expect(
        await api.appendRecordingEvents({
          xRecordingTabId: 101,
          xRecordingDocumentId: 'C18F47A71C2B7DEF81230123456789AE',
          xRecordingBatchId: 'contract-ended-independent',
          body: '{"ts":300,"type":3,"data":{}}\n',
        }),
      ).toEqual({ accepted: 1 })
    },
  },
  {
    name: 'unknown harness',
    async run({ api }) {
      await expectApiError(
        () => api.connectHarness({ harness: 'Unknown' as Harness }),
        404,
        'harness_not_found',
      )
    },
  },
  {
    name: 'missing session previews',
    async run({ api, zeroTabLiveSessionId, endedSessionId }) {
      for (const sessionId of [
        zeroTabLiveSessionId,
        endedSessionId,
        'missing',
      ]) {
        await expectApiError(
          () => api.getSessionPreview({ sessionId }),
          404,
          'preview_not_found',
        )
      }
    },
  },
  {
    name: 'session screenshot ownership failures',
    async run({ api, liveSessionId, secondLiveSessionId, screenshotIds }) {
      await expectApiError(
        () =>
          api.getSessionScreenshot({
            sessionId: liveSessionId,
            screenshotId: 999,
          }),
        404,
        'screenshot_not_found',
      )
      await expectApiError(
        () =>
          api.getSessionScreenshot({
            sessionId: secondLiveSessionId,
            screenshotId: screenshotIds[0],
          }),
        404,
        'screenshot_not_found',
      )
      await expectApiError(
        () => api.listSessionScreenshots({ sessionId: 'missing' }),
        404,
        'session_not_found',
      )
    },
  },
  {
    name: 'retired REST aliases',
    async run({ baseUrl }) {
      for (const [method, path] of RETIRED_ROUTES) {
        const response = await fetch(`${baseUrl}${path}`, { method })
        expect(response.status, `${method} ${path}`).toBe(404)
        expect(
          response.headers.get('content-type') ?? '',
          `${method} ${path}`,
        ).not.toContain('application/json')
      }
    },
  },
  {
    name: 'shutdown',
    async run({ api }) {
      expect(await api.shutdown()).toEqual({ status: 'ok' })
    },
  },
]

async function expectApiError(
  request: () => Promise<unknown>,
  status: number,
  code: string,
): Promise<void> {
  try {
    await request()
  } catch (error) {
    expect(error).toBeInstanceOf(ContractHttpError)
    const response = (error as ContractHttpError).response
    expect(response.status).toBe(status)
    const body = (await response.json()) as ApiError
    expect(body).toMatchObject({ code, message: expect.any(String) })
    if (!body.requestId) throw new Error('canonical error omitted requestId')
    expect(response.headers.get('x-request-id')).toBe(body.requestId)
    return
  }
  throw new Error(`expected ${status} ${code}`)
}

function recordingLineOfBytes(bytes: number, timestamp: number): string {
  const prefix = `{"ts":${timestamp.toString()},"type":2,"data":{"html":"`
  const suffix = '"}}'
  return `${prefix}${'x'.repeat(bytes - prefix.length - suffix.length)}${suffix}`
}
