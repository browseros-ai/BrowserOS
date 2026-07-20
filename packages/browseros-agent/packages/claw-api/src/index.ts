export * from './generated/index.js'

export const CLAW_API_PORT_DEFAULT = 9200
export const RECORDING_INGEST_MAX_BYTES = 4 * 1024 * 1024
/** Aggregate target used until a server explicitly advertises a larger ceiling. */
export const RECORDING_INGEST_FALLBACK_MAX_BYTES = 2 * 1024 * 1024
export const MCP_PATH = '/mcp'
export const BROWSEROS_MCP_SERVER_NAME = 'BrowserClaw'

export function canonicalMcpUrlForPort(port = CLAW_API_PORT_DEFAULT): string {
  return `http://127.0.0.1:${port}${MCP_PATH}`
}
