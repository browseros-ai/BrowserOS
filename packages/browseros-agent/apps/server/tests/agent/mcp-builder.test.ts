import { describe, expect, it } from 'bun:test'
import { buildInternalBrowserMcpSpec } from '../../src/agent/mcp-builder'
import { BROWSEROS_TOOL_LEASE_HEADER } from '../../src/api/routes/mcp'

describe('buildInternalBrowserMcpSpec', () => {
  it('builds the required read-only loopback MCP connection', () => {
    expect(
      buildInternalBrowserMcpSpec({
        serverPort: 32123,
        leaseToken: 'lease-token',
        readOnly: true,
      }),
    ).toEqual({
      name: 'browseros',
      url: 'http://127.0.0.1:32123/mcp?structured=1&read_only=1',
      transport: 'streamable-http',
      headers: { [BROWSEROS_TOOL_LEASE_HEADER]: 'lease-token' },
      required: true,
    })
  })

  it('does not request the narrow catalogue for an agent-mode lease', () => {
    expect(
      buildInternalBrowserMcpSpec({
        serverPort: 32123,
        leaseToken: 'lease-token',
        readOnly: false,
      }).url,
    ).toBe('http://127.0.0.1:32123/mcp?structured=1')
  })
})
