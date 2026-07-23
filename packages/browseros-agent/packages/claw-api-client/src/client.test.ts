import { describe, expect, it, mock } from 'bun:test'
import { Harness } from '@browseros/claw-api'
import {
  ApiResponseError,
  buildSessionPreviewUrl,
  buildSessionScreenshotUrl,
  ClawApiClient,
} from './index'

const baseUrl = 'http://127.0.0.1:9511'
const documentId = '33D25F3CF060E81B14070BC356FF1871'
const cockpitStats = {
  hasMeasuredStats: true,
  allTime: {
    browserClawTokenEstimate: 250_000,
    screenshotFirstTokenEstimate: 240_000,
    rawTokenSavingsEstimate: -10_000,
    humanTimeSavedMs: 7_200_000,
    sessionCount: 12,
    toolCallCount: 200,
  },
  last30Days: {
    browserClawTokenEstimate: 150_000,
    screenshotFirstTokenEstimate: 180_000,
    rawTokenSavingsEstimate: 30_000,
    humanTimeSavedMs: 3_600_000,
    sessionCount: 5,
    toolCallCount: 100,
  },
  last7Days: {
    browserClawTokenEstimate: 0,
    screenshotFirstTokenEstimate: 0,
    rawTokenSavingsEstimate: 0,
    humanTimeSavedMs: 0,
    sessionCount: 0,
    toolCallCount: 0,
  },
}

interface RecordedRequest {
  url: string
  method: string
  credentials: RequestCredentials
  headers: Headers
  body: string
}

function responseFor(request: Request): Response {
  const { pathname } = new URL(request.url)
  if (pathname.endsWith('/recording/events')) {
    return new Response('{"type":2}\n', {
      headers: { 'content-type': 'application/x-ndjson' },
    })
  }
  if (pathname.endsWith('/preview') || pathname.includes('/screenshots/')) {
    return new Response(new Uint8Array([0xff, 0xd8]), {
      headers: { 'content-type': 'image/jpeg' },
    })
  }
  if (pathname.endsWith('/screenshots')) {
    return Response.json({
      items: [{ screenshotId: 17, capturedAt: 123, toolName: 'act' }],
    })
  }
  if (pathname.endsWith('/sessions')) return Response.json({ items: [] })
  if (pathname.endsWith('/settings/telemetry')) {
    return Response.json({
      distinctId: 'install-1',
      enabled: false,
      consent: false,
    })
  }
  if (pathname.endsWith('/recordings/events')) {
    return Response.json({ accepted: 1 })
  }
  if (pathname.includes('/connections/')) {
    return Response.json({
      harness: Harness.Codex,
      installed: request.method === 'PUT',
      message: 'ok',
    })
  }
  return Response.json({ session: {}, dispatches: [] })
}

describe('ClawApiClient', () => {
  it('gets and unwraps cockpit stats from the exact route', async () => {
    const requests: Request[] = []
    const client = new ClawApiClient(`${baseUrl}/`, {
      fetch: async (input, init) => {
        const request =
          input instanceof Request ? input : new Request(input, init)
        requests.push(request)
        return Response.json(cockpitStats)
      },
    })

    await expect(client.getCockpitStats()).resolves.toEqual(cockpitStats)
    expect(requests).toHaveLength(1)
    expect(requests[0]?.method).toBe('GET')
    expect(new URL(requests[0]?.url ?? '').pathname).toBe(
      '/api/v1/cockpit/stats',
    )
  })

  it('preserves standard API errors for cockpit stats', async () => {
    const client = new ClawApiClient(baseUrl, {
      fetch: async () =>
        Response.json(
          {
            code: 'internal_error',
            message: 'failed to aggregate cockpit stats',
            requestId: 'request-stats',
          },
          { status: 500 },
        ),
    })

    await expect(client.getCockpitStats()).rejects.toBeInstanceOf(
      ApiResponseError,
    )
  })

  it('maps contract operations across JSON, NDJSON, text, blob, path, query, and headers', async () => {
    const requests: RecordedRequest[] = []
    const receivers: unknown[] = []
    async function receiverSensitiveFetch(
      this: void,
      input: Parameters<typeof globalThis.fetch>[0],
      init?: Parameters<typeof globalThis.fetch>[1],
    ): Promise<Response> {
      receivers.push(this)
      if (this !== undefined) throw new TypeError('Illegal invocation')
      const request =
        input instanceof Request ? input : new Request(input, init)
      requests.push({
        url: request.url,
        method: request.method,
        credentials: init?.credentials ?? request.credentials,
        headers: new Headers(request.headers),
        body: await request.clone().text(),
      })
      return responseFor(request)
    }

    const client = new ClawApiClient(baseUrl, {
      fetch: receiverSensitiveFetch,
    })

    await expect(
      client.listSessions({
        profileId: 'profile/one',
        search: 'agent work',
        status: 'live',
        since: 100,
        limit: 25,
      }),
    ).resolves.toEqual({ items: [] })
    await client.getSession({ sessionId: 'session / one' })
    await expect(
      client.updateTelemetry({
        updateTelemetryRequest: { consent: false },
      }),
    ).resolves.toMatchObject({ consent: false })
    const ndjson = '{"timestamp":100,"type":2,"data":{}}\n'
    await expect(
      client.appendRecordingEvents({
        xRecordingTabId: 101,
        xRecordingDocumentId: documentId,
        xRecordingBatchId: 'batch-1',
        xRecordingHasGap: true,
        body: ndjson,
      }),
    ).resolves.toEqual({ accepted: 1 })
    await expect(
      client.downloadRecordingEvents({ sessionId: 'session / one' }),
    ).resolves.toBe('{"type":2}\n')
    const preview = await client.getSessionPreview({
      sessionId: 'session / one',
      refresh: 456,
    })
    await expect(preview.arrayBuffer()).resolves.toHaveProperty('byteLength', 2)
    await expect(
      client.listSessionScreenshots({ sessionId: 'session / one' }),
    ).resolves.toMatchObject({ items: [{ screenshotId: 17 }] })
    const screenshot = await client.getSessionScreenshot({
      sessionId: 'session / one',
      screenshotId: 17,
    })
    expect(screenshot.type).toBe('image/jpeg')
    await client.connectHarness({ harness: Harness.Codex })
    await client.disconnectHarness({ harness: Harness.Codex })

    const listUrl = new URL(requests[0]?.url ?? '')
    expect(listUrl.pathname).toBe('/api/v1/sessions')
    expect(Object.fromEntries(listUrl.searchParams)).toEqual({
      profileId: 'profile/one',
      search: 'agent work',
      status: 'live',
      since: '100',
      limit: '25',
    })
    expect(new URL(requests[1]?.url ?? '').pathname).toBe(
      '/api/v1/sessions/session%20%2F%20one',
    )
    expect(requests[2]).toMatchObject({
      method: 'PUT',
      body: '{"consent":false}',
    })
    expect(requests[2]?.headers.get('content-type')).toBe('application/json')
    expect(requests[3]).toMatchObject({ method: 'POST', body: ndjson })
    expect(requests[3]?.headers.get('content-type')).toBe(
      'application/x-ndjson',
    )
    expect(requests[3]?.headers.get('x-recording-tab-id')).toBe('101')
    expect(requests[3]?.headers.get('x-recording-document-id')).toBe(documentId)
    expect(requests[3]?.headers.get('x-recording-batch-id')).toBe('batch-1')
    expect(requests[3]?.headers.get('x-recording-has-gap')).toBe('true')
    expect(new URL(requests[4]?.url ?? '').pathname).toEndWith(
      '/recording/events',
    )
    expect(new URL(requests[5]?.url ?? '').searchParams.get('refresh')).toBe(
      '456',
    )
    expect(requests.at(-2)?.method).toBe('PUT')
    expect(requests.at(-1)?.method).toBe('DELETE')
    expect(requests.every((request) => request.credentials === 'omit')).toBe(
      true,
    )
    expect(
      requests.every(
        (request) => request.headers.get('accept') !== 'text/html',
      ),
    ).toBe(true)
    expect(receivers).toEqual(requests.map(() => undefined))
  })

  it('preserves an unconsumed error response after openapi-fetch parses it', async () => {
    const client = new ClawApiClient(baseUrl, {
      fetch: async () =>
        Response.json(
          {
            code: 'session_not_found',
            message: 'session not found',
            requestId: 'request-1',
          },
          {
            status: 404,
            headers: { 'x-request-id': 'request-1' },
          },
        ),
    })

    try {
      await client.getSession({ sessionId: 'missing' })
      throw new Error('expected getSession to reject')
    } catch (error) {
      expect(error).toBeInstanceOf(ApiResponseError)
      const response = (error as ApiResponseError).response
      expect(response.status).toBe(404)
      expect(response.headers.get('content-type')).toContain('application/json')
      expect(response.headers.get('x-request-id')).toBe('request-1')
      await expect(response.json()).resolves.toEqual({
        code: 'session_not_found',
        message: 'session not found',
        requestId: 'request-1',
      })
    }
  })

  it('does not clone successful response bodies', async () => {
    const response = Response.json({ items: [] })
    const originalClone = response.clone.bind(response)
    const clone = mock(originalClone)
    response.clone = clone
    const client = new ClawApiClient(baseUrl, {
      fetch: async () => response,
    })

    await expect(client.listSessions()).resolves.toEqual({ items: [] })
    expect(clone).not.toHaveBeenCalled()
  })

  it('exposes all 18 facade operations', () => {
    const client = new ClawApiClient(baseUrl, {
      fetch: async () => Response.json({}),
    })
    const methods = [
      'getHealth',
      'shutdown',
      'getSystemInfo',
      'getCockpitStats',
      'getTelemetry',
      'updateTelemetry',
      'listSessions',
      'getSession',
      'cancelSession',
      'getRecording',
      'downloadRecordingEvents',
      'appendRecordingEvents',
      'getSessionPreview',
      'listSessionScreenshots',
      'getSessionScreenshot',
      'listConnections',
      'connectHarness',
      'disconnectHarness',
    ] as const

    expect(methods).toHaveLength(18)
    for (const method of methods) expect(client[method]).toBeFunction()
  })
})

describe('binary URL builders', () => {
  it('encodes path values and supports the optional preview refresh query', () => {
    expect(
      buildSessionPreviewUrl(`${baseUrl}/`, { sessionId: 'session / one' }),
    ).toBe(`${baseUrl}/api/v1/sessions/session%20%2F%20one/preview`)
    expect(
      buildSessionPreviewUrl(baseUrl, {
        sessionId: 'session / one',
        refresh: 0,
      }),
    ).toBe(`${baseUrl}/api/v1/sessions/session%20%2F%20one/preview?refresh=0`)
    expect(
      buildSessionScreenshotUrl(baseUrl, {
        sessionId: 'session / one',
        screenshotId: 17,
      }),
    ).toBe(`${baseUrl}/api/v1/sessions/session%20%2F%20one/screenshots/17`)
  })
})
