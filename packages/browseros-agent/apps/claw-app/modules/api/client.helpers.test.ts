import { afterEach, describe, expect, it } from 'bun:test'
import {
  resolveBrowserOSMcpBaseUrl,
  resolveBrowserOSServerBaseUrl,
} from './browseros-ports'
import {
  ApiResponseError,
  apiClient,
  apiClientForBaseUrl,
  ClawApiClient,
} from './client'
import { resolveApiBaseUrlFromSources } from './client.helpers'

const fallback = 'http://127.0.0.1:9200'
const originalChrome = globalThis.chrome
const originalFetch = globalThis.fetch
const originalWindow = globalThis.window

function installBrowserOSPrefs(values: Record<string, unknown>) {
  Object.defineProperty(globalThis, 'chrome', {
    configurable: true,
    value: {
      runtime: {},
      browserOS: {
        getPref(
          name: string,
          callback: (pref: chrome.browserOS.PrefObject) => void,
        ) {
          callback({
            key: name,
            type: typeof values[name],
            value: values[name],
          })
        },
      },
    },
  })
}

function installWindow(search: string, storage = new Map<string, string>()) {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
      location: { search },
      sessionStorage: {
        getItem(key: string) {
          return storage.get(key) ?? null
        },
        setItem(key: string, value: string) {
          storage.set(key, value)
        },
      },
    },
  })
}

function installFetchRecorder(options?: {
  unhealthyServerOrigins?: Set<string>
  unhealthyProxyOrigins?: Set<string>
}): string[] {
  const unhealthyServerOrigins =
    options?.unhealthyServerOrigins ?? new Set<string>()
  const unhealthyProxyOrigins =
    options?.unhealthyProxyOrigins ?? new Set<string>()
  const requests: string[] = []
  Object.defineProperty(globalThis, 'fetch', {
    configurable: true,
    value: async (input: Parameters<typeof fetch>[0]) => {
      const url = input instanceof Request ? input.url : String(input)
      requests.push(url)
      if (url.endsWith('/system/health')) {
        const origin = new URL(url).origin
        if (unhealthyServerOrigins.has(origin)) {
          return new Response('{}', { status: 503 })
        }
        return Response.json({ status: 'ok' })
      }
      if (url.endsWith('/health')) {
        const origin = new URL(url).origin
        if (unhealthyProxyOrigins.has(origin)) {
          return new Response('{}', { status: 503 })
        }
        return new Response('ok')
      }
      return new Response('{}', { status: 200 })
    },
  })
  return requests
}

afterEach(() => {
  Object.defineProperty(globalThis, 'chrome', {
    configurable: true,
    value: originalChrome,
  })
  Object.defineProperty(globalThis, 'fetch', {
    configurable: true,
    value: originalFetch,
  })
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: originalWindow,
  })
})

describe('resolveApiBaseUrlFromSources', () => {
  it('prefers the query override', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: 'http://127.0.0.1:9200',
        stored: 'http://127.0.0.1:9300',
        launcher: 'http://127.0.0.1:9400',
        fallback,
      }),
    ).toBe('http://127.0.0.1:9200')
  })

  it('uses session storage before the launcher env', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: null,
        stored: 'http://127.0.0.1:9300',
        launcher: 'http://127.0.0.1:9400',
        fallback,
      }),
    ).toBe('http://127.0.0.1:9300')
  })

  it('uses the launcher env before the default fallback', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: null,
        stored: null,
        launcher: 'http://127.0.0.1:9400',
        fallback,
      }),
    ).toBe('http://127.0.0.1:9400')
  })

  it('ignores non-loopback overrides', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: 'https://example.com',
        stored: 'http://localhost:9300',
        launcher: 'http://0.0.0.0:9400',
        fallback,
      }),
    ).toBe(fallback)
  })

  it('rejects loopback-looking URLs that parse to another host', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: 'http://127.0.0.1:@example.com',
        stored: null,
        launcher: null,
        fallback,
      }),
    ).toBe(fallback)
  })

  it('rejects malformed ports and pathful URLs', () => {
    expect(
      resolveApiBaseUrlFromSources({
        query: 'http://127.0.0.1:99999',
        stored: 'http://127.0.0.1:9300/cockpit',
        launcher: 'http://127.0.0.1:9400?x=1',
        fallback,
      }),
    ).toBe(fallback)
  })
})

describe('BrowserOS managed port resolution', () => {
  it('prefers the BrowserOS server port pref for API traffic', async () => {
    installBrowserOSPrefs({ 'browseros.server.server_port': 9511 })
    installFetchRecorder()

    await expect(
      resolveBrowserOSServerBaseUrl({
        query: 'http://127.0.0.1:9201',
        stored: 'http://127.0.0.1:9202',
        launcher: 'http://127.0.0.1:9203',
        fallback,
      }),
    ).resolves.toBe('http://127.0.0.1:9511')
  })

  it('prefers the BrowserOS proxy port pref for MCP traffic', async () => {
    installBrowserOSPrefs({ 'browseros.server.proxy_port': 9512 })
    const requests = installFetchRecorder({
      unhealthyProxyOrigins: new Set(['http://127.0.0.1:9512']),
    })

    await expect(
      resolveBrowserOSMcpBaseUrl({
        query: 'http://127.0.0.1:9201',
        stored: 'http://127.0.0.1:9202',
        launcher: 'http://127.0.0.1:9203',
        fallback,
      }),
    ).resolves.toBe('http://127.0.0.1:9512')
    expect(requests).toEqual([])
  })

  it('falls back to trusted sources when the pref is invalid', async () => {
    installBrowserOSPrefs({ 'browseros.server.server_port': '9511' })

    await expect(
      resolveBrowserOSServerBaseUrl({
        query: null,
        stored: 'http://127.0.0.1:9202',
        launcher: 'http://127.0.0.1:9203',
        fallback,
      }),
    ).resolves.toBe('http://127.0.0.1:9202')
  })

  it('keeps a valid server port pref when startup health is not ready yet', async () => {
    installBrowserOSPrefs({ 'browseros.server.server_port': 9511 })
    const requests = installFetchRecorder({
      unhealthyServerOrigins: new Set(['http://127.0.0.1:9511']),
    })

    await expect(
      resolveBrowserOSServerBaseUrl({
        query: null,
        stored: 'http://127.0.0.1:9202',
        launcher: 'http://127.0.0.1:9203',
        fallback,
      }),
    ).resolves.toBe('http://127.0.0.1:9511')
    expect(requests).toEqual([])
  })

  it('keeps a valid proxy port pref when startup health is not ready yet', async () => {
    installBrowserOSPrefs({ 'browseros.server.proxy_port': 9000 })
    const requests = installFetchRecorder({
      unhealthyProxyOrigins: new Set(['http://127.0.0.1:9000']),
    })

    await expect(
      resolveBrowserOSMcpBaseUrl({
        query: null,
        stored: null,
        launcher: null,
        fallback,
      }),
    ).resolves.toBe('http://127.0.0.1:9000')
    expect(requests).toEqual([])
  })

  it('routes typed API calls through the BrowserOS server port pref', async () => {
    installBrowserOSPrefs({ 'browseros.server.server_port': 9511 })
    const requests = installFetchRecorder()

    const response = await (await apiClient()).getHealth()

    expect(response).toEqual({ status: 'ok' })
    expect(requests).toEqual(['http://127.0.0.1:9511/system/health'])
  })

  it('routes typed API calls through trusted fallbacks when the pref is invalid', async () => {
    installWindow('?apiUrl=http%3A%2F%2F127.0.0.1%3A9432')
    installBrowserOSPrefs({ 'browseros.server.server_port': '9511' })
    const requests = installFetchRecorder()

    const response = await (await apiClient()).getHealth()

    expect(response).toEqual({ status: 'ok' })
    expect(requests).toEqual(['http://127.0.0.1:9432/system/health'])
  })

  it('reuses one typed client per resolved base URL', () => {
    const first = apiClientForBaseUrl('http://127.0.0.1:9200')
    expect(apiClientForBaseUrl('http://127.0.0.1:9200')).toBe(first)
    expect(apiClientForBaseUrl('http://127.0.0.1:9300')).not.toBe(first)
  })

  it('maps typed JSON and NDJSON calls to the canonical wire contract', async () => {
    const requests: Array<{ url: string; init?: RequestInit }> = []
    const client = new ClawApiClient('http://127.0.0.1:9200', {
      fetch: async (input, init) => {
        const url = input instanceof Request ? input.url : String(input)
        requests.push({ url, init })
        if (url.includes('/api/v1/sessions?')) {
          return Response.json({ items: [] })
        }
        if (url.endsWith('/api/v1/settings/telemetry')) {
          return Response.json({
            distinctId: 'install-1',
            enabled: false,
            consent: false,
          })
        }
        return Response.json({ accepted: 1 })
      },
    })

    await expect(
      client.listSessions({
        profileId: 'profile/one',
        status: 'live',
        since: 100,
        limit: 25,
      }),
    ).resolves.toEqual({ items: [] })
    await expect(
      client.updateTelemetry({ updateTelemetryRequest: { consent: false } }),
    ).resolves.toMatchObject({ consent: false })
    await expect(
      client.appendRecordingEvents({
        xRecordingTabId: 101,
        xRecordingDocumentId: '33D25F3CF060E81B14070BC356FF1871',
        xRecordingBatchId: 'batch-1',
        xRecordingHasGap: true,
        body: '{"ts":100,"type":2,"data":{}}\n',
      }),
    ).resolves.toEqual({ accepted: 1 })

    expect(requests[0]?.url).toBe(
      'http://127.0.0.1:9200/api/v1/sessions?profileId=profile%2Fone&status=live&since=100&limit=25',
    )
    expect(requests[1]?.init).toMatchObject({
      method: 'PUT',
      credentials: 'omit',
      body: '{"consent":false}',
    })
    expect(new Headers(requests[1]?.init?.headers).get('content-type')).toBe(
      'application/json',
    )
    expect(requests[2]?.init).toMatchObject({
      method: 'POST',
      credentials: 'omit',
    })
    const recordingHeaders = new Headers(requests[2]?.init?.headers)
    expect(recordingHeaders.get('content-type')).toBe('application/x-ndjson')
    expect(recordingHeaders.get('x-recording-tab-id')).toBe('101')
    expect(recordingHeaders.get('x-recording-document-id')).toBe(
      '33D25F3CF060E81B14070BC356FF1871',
    )
    expect(recordingHeaders.get('x-recording-batch-id')).toBe('batch-1')
    expect(recordingHeaders.get('x-recording-has-gap')).toBe('true')
  })

  it('returns JPEG blobs and preserves generated ApiError response bodies', async () => {
    const client = new ClawApiClient('http://127.0.0.1:9200', {
      fetch: async (input) => {
        const url = input instanceof Request ? input.url : String(input)
        if (url.endsWith('/preview')) {
          return new Response(new Uint8Array([0xff, 0xd8]), {
            headers: { 'content-type': 'image/jpeg' },
          })
        }
        return Response.json(
          {
            code: 'session_not_found',
            message: 'session not found',
            requestId: 'request-1',
          },
          { status: 404 },
        )
      },
    })

    const preview = await client.getSessionPreview({
      sessionId: 'session/one',
    })
    expect(preview.type).toBe('image/jpeg')
    expect(Array.from(new Uint8Array(await preview.arrayBuffer()))).toEqual([
      0xff, 0xd8,
    ])

    try {
      await client.getSession({ sessionId: 'missing' })
      throw new Error('expected getSession to reject')
    } catch (error) {
      expect(error).toBeInstanceOf(ApiResponseError)
      const response = (error as ApiResponseError).response
      expect(response.status).toBe(404)
      await expect(response.json()).resolves.toEqual({
        code: 'session_not_found',
        message: 'session not found',
        requestId: 'request-1',
      })
    }
  })

  it('maps session visual calls to session-owned routes', async () => {
    const requests: string[] = []
    const client = new ClawApiClient('http://127.0.0.1:9200', {
      fetch: async (input) => {
        const url = input instanceof Request ? input.url : String(input)
        requests.push(url)
        if (url.endsWith('/screenshots')) {
          return Response.json({
            items: [{ screenshotId: 17, capturedAt: 123, toolName: 'act' }],
          })
        }
        return new Response(new Uint8Array([0xff, 0xd8]), {
          headers: { 'content-type': 'image/jpeg' },
        })
      },
    })

    await client.getSessionPreview({ sessionId: 'session/one' })
    await expect(
      client.listSessionScreenshots({ sessionId: 'session/one' }),
    ).resolves.toEqual({
      items: [{ screenshotId: 17, capturedAt: 123, toolName: 'act' }],
    })
    await client.getSessionScreenshot({
      sessionId: 'session/one',
      screenshotId: 17,
    })

    expect(requests).toEqual([
      'http://127.0.0.1:9200/api/v1/sessions/session%2Fone/preview',
      'http://127.0.0.1:9200/api/v1/sessions/session%2Fone/screenshots',
      'http://127.0.0.1:9200/api/v1/sessions/session%2Fone/screenshots/17',
    ])
  })
})
