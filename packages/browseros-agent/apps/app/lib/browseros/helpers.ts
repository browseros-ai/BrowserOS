import { getBrowserOSAdapter } from './adapter'
import { BROWSEROS_PREFS } from './prefs'

class McpPortError extends Error {
  constructor() {
    super('MCP server port not configured.')
    this.name = 'McpPortError'
  }
}

/**
 * Returns the local BrowserOS server base URL for chat and agent APIs.
 * BrowserOS publishes this through the unified MCP/server-port preference.
 */
export async function getAgentServerUrl(): Promise<string> {
  const port = await getMcpPort()
  return `http://127.0.0.1:${port}`
}

async function getMcpPort(): Promise<number> {
  try {
    const adapter = getBrowserOSAdapter()
    const pref = await adapter.getPref(BROWSEROS_PREFS.MCP_PORT)

    if (pref?.value && typeof pref.value === 'number') {
      return pref.value
    }
  } catch {
    // BrowserOS API not available
  }

  throw new McpPortError()
}
