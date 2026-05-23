import { env } from '@/lib/env'
import { getBrowserOSAdapter } from './adapter'
import { Capabilities, Feature } from './capabilities'
import { BROWSEROS_PREFS } from './prefs'

export class AgentPortError extends Error {
  constructor() {
    super('Agent server port not configured.')
    this.name = 'AgentPortError'
  }
}

export class McpPortError extends Error {
  constructor() {
    super('MCP server port not configured.')
    this.name = 'McpPortError'
  }
}

/**
 * @public
 */
export async function getAgentServerUrl(): Promise<string> {
  const supportsUnifiedPort = await Capabilities.supports(
    Feature.UNIFIED_PORT_SUPPORT,
  )
  if (supportsUnifiedPort) {
    const port = await getMcpPort()
    return `http://127.0.0.1:${port}`
  }
  const port = await getAgentPort()
  return `http://127.0.0.1:${port}`
}

async function getAgentPort(): Promise<number> {
  if (env.VITE_BROWSEROS_SERVER_PORT) {
    return env.VITE_BROWSEROS_SERVER_PORT
  }

  try {
    const adapter = getBrowserOSAdapter()
    const pref = await adapter.getPref(BROWSEROS_PREFS.AGENT_PORT)

    if (pref?.value && typeof pref.value === 'number') {
      return pref.value
    }
  } catch {
    // BrowserOS API not available
  }

  throw new AgentPortError()
}

async function getMcpPort(): Promise<number> {
  // In dev mode, fall back to env var when browser prefs aren't available
  if (env.VITE_BROWSEROS_SERVER_PORT) {
    return env.VITE_BROWSEROS_SERVER_PORT
  }

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

/**
 * @public
 */
export async function getMcpServerUrl(): Promise<string> {
  const supportsProxy = await Capabilities.supports(Feature.PROXY_SUPPORT)
  if (supportsProxy) {
    const port = await getProxyPort()
    return `http://127.0.0.1:${port}/mcp`
  }
  const port = await getMcpPort()
  return `http://127.0.0.1:${port}/mcp`
}

export class ProxyPortError extends Error {
  constructor() {
    super('Proxy server port not configured.')
    this.name = 'ProxyPortError'
  }
}

async function getProxyPort(): Promise<number> {
  try {
    const adapter = getBrowserOSAdapter()
    const pref = await adapter.getPref(BROWSEROS_PREFS.PROXY_PORT)

    if (pref?.value && typeof pref.value === 'number') {
      return pref.value
    }
  } catch {
    // BrowserOS API not available
  }

  throw new ProxyPortError()
}

/**
 * @public
 */
export async function getProxyServerUrl(): Promise<string> {
  const port = await getProxyPort()
  return `http://127.0.0.1:${port}`
}

/**
 * @public
 */
export async function getHealthCheckUrl(): Promise<string> {
  const supportsProxy = await Capabilities.supports(Feature.PROXY_SUPPORT)
  if (supportsProxy) {
    const port = await getProxyPort()
    return `http://127.0.0.1:${port}/health`
  }
  const port = await getMcpPort()
  return `http://127.0.0.1:${port}/health`
}
