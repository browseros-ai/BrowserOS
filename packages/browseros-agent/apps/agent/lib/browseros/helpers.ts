import { env } from '@/lib/env'
import { getBrowserOSAdapter } from './adapter'
import { Capabilities, Feature } from './capabilities'
import { BROWSEROS_PREFS } from './prefs'

/**
 * Prefer the port embedded in VITE_PUBLIC_BROWSEROS_API so chat/MCP/health use the
 * same origin as GraphQL without keeping VITE_BROWSEROS_SERVER_PORT in sync manually.
 */
function getPortFromPublicBrowserosApi(): number | undefined {
  const raw = env.VITE_PUBLIC_BROWSEROS_API?.trim()
  if (!raw) return undefined
  try {
    const u = new URL(raw)
    if (u.port !== '') {
      const n = Number.parseInt(u.port, 10)
      return Number.isNaN(n) ? undefined : n
    }
  } catch {
    return undefined
  }
  return undefined
}

function getConfiguredLocalServerPort(): number | undefined {
  const fromApi = getPortFromPublicBrowserosApi()
  if (fromApi !== undefined) return fromApi
  if (env.VITE_BROWSEROS_SERVER_PORT !== undefined) {
    return env.VITE_BROWSEROS_SERVER_PORT
  }
  return undefined
}

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
  const configured = getConfiguredLocalServerPort()
  if (configured !== undefined) {
    return configured
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
  const configured = getConfiguredLocalServerPort()
  if (configured !== undefined) {
    return configured
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
  const configured = getConfiguredLocalServerPort()
  if (configured !== undefined) {
    return configured
  }

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
