import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'

const MCP_PORT_PREF = 'browseros.server.mcp_port'
let originalChrome: typeof globalThis.chrome | undefined

function readPref(name: string): { value: unknown } {
  if (name === MCP_PORT_PREF) return { value: 9105 }
  return { value: null }
}

mock.module('./prefs', () => ({
  BROWSEROS_PREFS: {
    MCP_PORT: MCP_PORT_PREF,
    PROVIDERS: 'browseros.providers',
    THIRD_PARTY_LLM_PROVIDERS: 'browseros.third_party_llm.providers',
    SERVER_PORT: 'browseros.server.server_port',
    SHOW_LLM_CHAT: 'browseros.show_llm_chat',
    SHOW_TOOLBAR_LABELS: 'browseros.show_toolbar_labels',
    VERTICAL_TABS_ENABLED: 'browseros.vertical_tabs_enabled',
    INSTALL_ID: 'browseros.metrics_install_id',
  },
}))

mock.module('./adapter', () => ({
  BrowserOSAdapter: {
    getInstance: () => ({
      getPref: async (name: string) => readPref(name),
      getBrowserosVersion: async () => null,
    }),
  },
  getBrowserOSAdapter: () => ({
    getPref: async (name: string) => readPref(name),
  }),
}))

describe('BrowserOS helper URLs', () => {
  beforeEach(() => {
    originalChrome = globalThis.chrome
    Object.assign(globalThis, {
      chrome: {
        ...originalChrome,
        browserOS: {
          ...originalChrome?.browserOS,
          getPref: (
            name: string,
            resolve: (result: { value: unknown }) => void,
          ) => {
            resolve(readPref(name))
          },
        },
      },
    })
  })

  afterEach(() => {
    if (originalChrome) {
      Object.assign(globalThis, { chrome: originalChrome })
      return
    }
    Reflect.deleteProperty(globalThis, 'chrome')
  })

  it('uses the BrowserOS MCP port as the server URL', async () => {
    const { getAgentServerUrl } = await import('./helpers')

    await expect(getAgentServerUrl()).resolves.toBe('http://127.0.0.1:9105')
  })
})
