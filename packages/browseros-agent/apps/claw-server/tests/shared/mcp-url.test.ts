import { describe, expect, it } from 'bun:test'
import {
  BROWSEROS_MCP_SERVER_NAME,
  canonicalMcpUrlForPort,
  MCP_PATH,
} from '../../src/shared/mcp-url'

describe('canonicalMcpUrlForPort', () => {
  it('emits the slugless v2 shape on the default standalone port', () => {
    expect(canonicalMcpUrlForPort()).toBe('http://127.0.0.1:9200/cockpit/mcp')
  })

  it('respects an alternate dev port', () => {
    expect(canonicalMcpUrlForPort(9100)).toBe(
      'http://127.0.0.1:9100/cockpit/mcp',
    )
  })

  it('uses the constant mcp path', () => {
    expect(MCP_PATH).toBe('/mcp')
  })
})

describe('BROWSEROS_MCP_SERVER_NAME', () => {
  it('is the canonical "browseros" key written into harness configs', () => {
    expect(BROWSEROS_MCP_SERVER_NAME).toBe('browseros')
  })
})
