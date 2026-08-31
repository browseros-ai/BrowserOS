import { describe, expect, it, mock } from 'bun:test'
import {
  BROWSEROS_TOOL_LEASE_HEADER,
  createMcpRoutes,
} from '../../../src/api/routes/mcp'
import { InvalidBrowserToolLeaseError } from '../../../src/api/services/mcp/browser-mcp-module'

class FakeTransport {
  handleRequest = async () => Response.json({ ok: true })
}

async function post(
  app: ReturnType<typeof createMcpRoutes>,
  path: string,
  headers: Record<string, string> = {},
) {
  return app.request(path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', ...headers },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list' }),
  })
}

describe('/mcp BrowserMcpModule adapter', () => {
  it('passes the lease and read-only request to the module', async () => {
    const createMcpServer = mock(() => ({
      connect: async () => undefined,
    }))
    const app = createMcpRoutes({
      browserMcp: {
        createMcpServer,
        validateLeaseToken: mock(() => {}),
      } as never,
      createMcpTransport: (() => new FakeTransport()) as never,
    })

    const response = await post(app, '/?read_only=1&structured=1', {
      [BROWSEROS_TOOL_LEASE_HEADER]: 'lease-1',
    })

    expect(response.status).toBe(200)
    expect(createMcpServer).toHaveBeenCalledWith({
      leaseToken: 'lease-1',
      requestedReadOnly: true,
      includeStructuredContent: true,
    })
  })

  it('rejects an invalid capability at transport admission', async () => {
    const createMcpServer = mock(() => ({ connect: async () => undefined }))
    const validateLeaseToken = mock(() => {
      throw new InvalidBrowserToolLeaseError()
    })
    const app = createMcpRoutes({
      browserMcp: { createMcpServer, validateLeaseToken } as never,
      createMcpTransport: (() => new FakeTransport()) as never,
    })

    const response = await post(app, '/', {
      [BROWSEROS_TOOL_LEASE_HEADER]: 'expired',
    })

    expect(response.status).toBe(401)
    expect(validateLeaseToken).toHaveBeenCalledWith('expired')
    expect(createMcpServer).not.toHaveBeenCalled()
  })
})
