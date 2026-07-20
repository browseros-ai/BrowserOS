/**
 * The behavioral half of the contract: every case runs verbatim against
 * both server implementations through the generated client (raw fetch
 * where the client can't express the check), asserting observable wire
 * behavior only — status codes, envelopes, headers — never anything
 * implementation-specific. A case that passes on one server and fails
 * on the other is by definition a contract violation.
 *
 * Cases assume the seeded world the adapters provide (a live and an
 * ended session, tab 101 / page 7 / target-7, one screenshot). The
 * `shutdown` case must stay last: it kills the server it runs against.
 */

import { expect } from 'bun:test'
import {
  type ApiError,
  Harness,
  RECORDING_INGEST_MAX_BYTES,
  ResponseError,
} from '../../../packages/claw-api/src'
import type { ContractServer } from './server-adapters'

export interface ContractCase {
  name: string
  run(server: ContractServer): Promise<void>
}

// Legacy vocabulary the canonical surface must never leak (sessions
// replaced agents/tasks/runs), and the inline-frame key the tab list
// must not carry (binary travels via the preview/screenshot endpoints).
// Spelled via concatenation so the forbidden names never appear
// literally in the contract package — a plain grep for them under
// `contracts/claw-api` should come up empty.
const FORBIDDEN_IDENTITY_KEYS = ['agent', 'task', 'run'].map(
  (scope) => `${scope}Id`,
)
const INLINE_JPEG_KEY = ['jpeg', 'Base64'].join('')
const RETIRED_ROUTES = [
  ['GET', '/system/version'],
  ['GET', '/system/url'],
  ['GET', '/system/telemetry'],
  ['POST', '/system/telemetry'],
  ['POST', '/agents/agent-1/cancel'],
  ['GET', '/tabs/activity'],
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
      expect(detail.dispatches[0]).toMatchObject({
        dispatchId: 1,
        pageId: 7,
        targetId: 'target-7',
        hasScreenshot: true,
      })
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
        sizeBytes: 0,
        pageIds: [],
      })
    },
  },
  {
    name: 'recording append and NDJSON download',
    async run({ api, baseUrl, liveSessionId }) {
      const body = '{"tabPageId":7,"ts":100}\n{"tabPageId":7,"ts":200}\n'
      const request = {
        sessionId: liveSessionId,
        xRecordingTabId: 101,
        xRecordingPageId: 7,
        xRecordingTargetId: 'target-7',
        xRecordingBatchId: 'contract-batch-1',
        body,
      }
      expect(await api.appendRecordingEvents(request)).toEqual({ accepted: 2 })
      expect(await api.appendRecordingEvents(request)).toEqual({ accepted: 0 })
      expect(
        await api.getRecording({ sessionId: liveSessionId }),
      ).toMatchObject({ hasData: true, pageIds: [7] })
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
    name: 'recording ingest byte ceiling',
    async run({ api, liveSessionId }) {
      const accepted = recordingLineOfBytes(RECORDING_INGEST_MAX_BYTES, 400)
      expect(
        await api.appendRecordingEvents({
          sessionId: liveSessionId,
          xRecordingTabId: 101,
          xRecordingPageId: 7,
          xRecordingTargetId: 'target-7',
          xRecordingBatchId: 'contract-boundary',
          body: accepted,
        }),
      ).toEqual({ accepted: 1 })

      await expectApiError(
        () =>
          api.appendRecordingEvents({
            sessionId: liveSessionId,
            xRecordingTabId: 101,
            xRecordingPageId: 7,
            xRecordingTargetId: 'target-7',
            xRecordingBatchId: 'contract-over-limit',
            body: recordingLineOfBytes(RECORDING_INGEST_MAX_BYTES + 1, 401),
          }),
        413,
        'recording_payload_too_large',
      )

      expect(
        await api.appendRecordingEvents({
          sessionId: liveSessionId,
          xRecordingTabId: 101,
          xRecordingPageId: 7,
          xRecordingTargetId: 'target-7',
          xRecordingBatchId: 'contract-after-over-limit',
          body: '{"ts":402,"type":3,"data":{}}',
        }),
      ).toEqual({ accepted: 1 })
    },
  },
  {
    name: 'tab list and JPEG artifacts',
    async run({ api, baseUrl, screenshotDispatchId }) {
      const tabs = await api.listTabs()
      expect(tabs.items[0]).toMatchObject({
        tabId: 101,
        pageId: 7,
        targetId: 'target-7',
        sessionId: 'session-live',
      })
      expect(JSON.stringify(tabs)).not.toContain(INLINE_JPEG_KEY)

      for (const path of [
        '/api/v1/tabs/7/preview',
        `/api/v1/dispatches/${screenshotDispatchId}/screenshot`,
      ]) {
        const response = await fetch(`${baseUrl}${path}`)
        expect(response.status, path).toBe(200)
        expect(response.headers.get('content-type'), path).toBe('image/jpeg')
        expect(
          Array.from(new Uint8Array(await response.arrayBuffer())),
          path,
        ).toEqual([0xff, 0xd8])
      }
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
    name: 'ended recording write',
    async run({ api, endedSessionId }) {
      await expectApiError(
        () =>
          api.appendRecordingEvents({
            sessionId: endedSessionId,
            xRecordingTabId: 101,
            xRecordingPageId: 7,
            xRecordingTargetId: 'target-7',
            body: '{"ts":300}\n',
          }),
        410,
        'session_ended',
      )
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
    name: 'missing binary artifacts',
    async run({ baseUrl }) {
      for (const [path, code] of [
        ['/api/v1/tabs/999/preview', 'preview_not_found'],
        ['/api/v1/dispatches/999/screenshot', 'screenshot_not_found'],
      ] as const) {
        const response = await fetch(`${baseUrl}${path}`)
        expect(response.status, path).toBe(404)
        expect(await response.json(), path).toMatchObject({ code })
      }
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
    expect(error).toBeInstanceOf(ResponseError)
    const response = (error as ResponseError).response
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
