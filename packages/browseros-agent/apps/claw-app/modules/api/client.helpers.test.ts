import { afterEach, describe, expect, it } from 'bun:test'
import {
  resolveBrowserOSMcpBaseUrl,
  resolveBrowserOSServerBaseUrl,
} from './browseros-ports'
import { api } from './client'
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

  it('routes Hono API calls through the BrowserOS server port pref', async () => {
    installBrowserOSPrefs({ 'browseros.server.server_port': 9511 })
    const requests = installFetchRecorder()

    const response = await api.system.health.$get()

    expect(response.status).toBe(200)
    expect(requests).toEqual(['http://127.0.0.1:9511/system/health'])
  })

  it('routes Hono API calls through trusted fallbacks when the pref is invalid', async () => {
    installWindow('?apiUrl=http%3A%2F%2F127.0.0.1%3A9432')
    installBrowserOSPrefs({ 'browseros.server.server_port': '9511' })
    const requests = installFetchRecorder()

    const response = await api.system.health.$get()

    expect(response.status).toBe(200)
    expect(requests).toEqual(['http://127.0.0.1:9432/system/health'])
  })
})
