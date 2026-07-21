import { describe, expect, test } from 'bun:test'
import {
  RECORDING_INGEST_FALLBACK_MAX_BYTES,
  RECORDING_INGEST_MAX_BYTES,
} from './limits'
import { CLAW_API_PORT_DEFAULT, CLAW_CDP_PORT_DEFAULT } from './ports'
import {
  BROWSEROS_MCP_SERVER_NAME,
  canonicalMcpUrlForPort,
  MCP_PATH,
} from './urls'

describe('BrowserClaw runtime constants', () => {
  test('preserves standalone API and private CDP ports', () => {
    expect(CLAW_API_PORT_DEFAULT).toBe(9200)
    expect(CLAW_CDP_PORT_DEFAULT).toBe(49337)
  })

  test('preserves recording ingest ceilings', () => {
    expect(RECORDING_INGEST_MAX_BYTES).toBe(16 * 1024 * 1024)
    expect(RECORDING_INGEST_FALLBACK_MAX_BYTES).toBe(2 * 1024 * 1024)
  })

  test('preserves the canonical MCP identity and loopback URL', () => {
    expect(MCP_PATH).toBe('/mcp')
    expect(BROWSEROS_MCP_SERVER_NAME).toBe('BrowserClaw')
    expect(canonicalMcpUrlForPort()).toBe('http://127.0.0.1:9200/mcp')
    expect(canonicalMcpUrlForPort(9100)).toBe('http://127.0.0.1:9100/mcp')
  })
})
