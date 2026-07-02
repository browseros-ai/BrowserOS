import { afterEach, describe, expect, it } from 'bun:test'
import { API_URL_STORAGE_KEY } from './client.helpers'
import {
  buildCanonicalMcpCliCommand,
  buildCanonicalMcpEndpointUrl,
  buildMcpEndpointUrl,
  resolveCanonicalMcpEndpointUrl,
  resolveMcpEndpointUrl,
} from './mcp-endpoint'

const originalWindow = globalThis.window
const originalChrome = globalThis.chrome

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
  return storage
}

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

afterEach(() => {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: originalWindow,
  })
  Object.defineProperty(globalThis, 'chrome', {
    configurable: true,
    value: originalChrome,
  })
})

describe('buildMcpEndpointUrl', () => {
  it('persists a valid query api URL for later same-session calls', () => {
    const storage = installWindow('?apiUrl=http%3A%2F%2F127.0.0.1%3A9234')

    expect(buildMcpEndpointUrl('demo')).toBe('http://127.0.0.1:9234/mcp/demo')
    expect(storage.get(API_URL_STORAGE_KEY)).toBe('http://127.0.0.1:9234')
  })

  it('uses the cached API URL when the query is absent', () => {
    const storage = new Map([[API_URL_STORAGE_KEY, 'http://127.0.0.1:9345']])
    installWindow('', storage)

    expect(buildMcpEndpointUrl('demo')).toBe('http://127.0.0.1:9345/mcp/demo')
  })

  it('uses the BrowserOS proxy port pref when available', async () => {
    installWindow('')
    installBrowserOSPrefs({ 'browseros.server.proxy_port': 9512 })

    await expect(resolveMcpEndpointUrl('demo')).resolves.toBe(
      'http://127.0.0.1:9512/mcp/demo',
    )
  })
})

describe('buildCanonicalMcpEndpointUrl', () => {
  it('emits the v2 slugless URL using the query-supplied base', () => {
    installWindow('?apiUrl=http%3A%2F%2F127.0.0.1%3A9234')
    expect(buildCanonicalMcpEndpointUrl()).toBe('http://127.0.0.1:9234/mcp')
  })

  it('falls back to the prod port root when no overrides exist', () => {
    installWindow('')
    expect(buildCanonicalMcpEndpointUrl()).toBe('http://127.0.0.1:9200/mcp')
  })

  it('uses the BrowserOS proxy port pref when available', async () => {
    installWindow('')
    installBrowserOSPrefs({ 'browseros.server.proxy_port': 9512 })

    await expect(resolveCanonicalMcpEndpointUrl()).resolves.toBe(
      'http://127.0.0.1:9512/mcp',
    )
  })
})

describe('buildCanonicalMcpCliCommand', () => {
  it('produces the standard `claude mcp add` shape with the canonical URL', () => {
    installWindow('')
    const cli = buildCanonicalMcpCliCommand()
    expect(cli).toContain('claude mcp add BrowserClaw')
    expect(cli).toContain('http://127.0.0.1:9200/mcp')
    expect(cli).toContain('--transport http')
    expect(cli).toContain('--scope user')
  })
})
