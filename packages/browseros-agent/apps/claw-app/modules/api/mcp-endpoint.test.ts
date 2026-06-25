import { afterEach, describe, expect, it } from 'bun:test'
import { API_URL_STORAGE_KEY } from './client.helpers'
import {
  buildCanonicalMcpCliCommand,
  buildCanonicalMcpEndpointUrl,
  buildMcpEndpointUrl,
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

describe('buildMcpEndpointUrl', () => {
  it('persists a valid query api URL for later same-session calls', () => {
    const storage = installWindow(
      '?apiUrl=http%3A%2F%2F127.0.0.1%3A9234%2Fcockpit',
    )

    expect(buildMcpEndpointUrl('demo')).toBe(
      'http://127.0.0.1:9234/cockpit/mcp/demo',
    )
    expect(storage.get(API_URL_STORAGE_KEY)).toBe(
      'http://127.0.0.1:9234/cockpit',
    )
  })

  it('uses the cached API URL when the query is absent', () => {
    const storage = new Map([
      [API_URL_STORAGE_KEY, 'http://127.0.0.1:9345/cockpit'],
    ])
    installWindow('', storage)

    expect(buildMcpEndpointUrl('demo')).toBe(
      'http://127.0.0.1:9345/cockpit/mcp/demo',
    )
  })
})

describe('buildCanonicalMcpEndpointUrl', () => {
  it('emits the v2 slugless URL using the query-supplied base', () => {
    installWindow('?apiUrl=http%3A%2F%2F127.0.0.1%3A9234%2Fcockpit')
    expect(buildCanonicalMcpEndpointUrl()).toBe(
      'http://127.0.0.1:9234/cockpit/mcp',
    )
  })

  it('falls back to the prod port + cockpit prefix when no overrides exist', () => {
    installWindow('')
    expect(buildCanonicalMcpEndpointUrl()).toBe(
      'http://127.0.0.1:9200/cockpit/mcp',
    )
  })
})

describe('buildCanonicalMcpCliCommand', () => {
  it('produces the standard `claude mcp add` shape with the canonical URL', () => {
    installWindow('')
    const cli = buildCanonicalMcpCliCommand()
    expect(cli).toContain('claude mcp add browseros')
    expect(cli).toContain('http://127.0.0.1:9200/cockpit/mcp')
    expect(cli).toContain('--transport http')
    expect(cli).toContain('--scope user')
  })
})
