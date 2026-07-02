import { afterEach, describe, expect, it } from 'bun:test'
import { API_URL_STORAGE_KEY } from './client.helpers'
import {
  buildCanonicalMcpCliCommand,
  buildCanonicalMcpEndpointUrl,
  buildCockpitHomeUrl,
} from './mcp-endpoint'

const originalWindow = globalThis.window

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

afterEach(() => {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: originalWindow,
  })
})

describe('buildCockpitHomeUrl', () => {
  it('persists a valid query API URL for later same-session calls', () => {
    const storage = installWindow('?apiUrl=http%3A%2F%2F127.0.0.1%3A9234')

    expect(buildCockpitHomeUrl()).toBe('http://127.0.0.1:9234')
    expect(storage.get(API_URL_STORAGE_KEY)).toBe('http://127.0.0.1:9234')
  })

  it('uses the cached API URL when the query is absent', () => {
    const storage = new Map([[API_URL_STORAGE_KEY, 'http://127.0.0.1:9345']])
    installWindow('', storage)

    expect(buildCockpitHomeUrl()).toBe('http://127.0.0.1:9345')
  })

  it('falls back to the prod port root when no overrides exist', () => {
    installWindow('')
    expect(buildCockpitHomeUrl()).toBe('http://127.0.0.1:9200')
  })
})

describe('buildCanonicalMcpEndpointUrl', () => {
  it('emits the v2 slugless URL using the resolved API base', () => {
    installWindow('?apiUrl=http%3A%2F%2F127.0.0.1%3A9234')
    expect(buildCanonicalMcpEndpointUrl()).toBe('http://127.0.0.1:9234/mcp')
  })
})

describe('buildCanonicalMcpCliCommand', () => {
  it('produces the standard Claude MCP registration command', () => {
    installWindow('')
    const cli = buildCanonicalMcpCliCommand()
    expect(cli).toContain('claude mcp add BrowserClaw')
    expect(cli).toContain('http://127.0.0.1:9200/mcp')
    expect(cli).toContain('--transport http')
    expect(cli).toContain('--scope user')
  })
})
