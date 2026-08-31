import { describe, expect, it, mock } from 'bun:test'
import {
  BROWSEROS_TOOL_LEASE_HEADER,
  createMcpRoutes,
} from '../../../src/api/routes/mcp'

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

describe('/mcp BrowserToolRuntime adapter', () => {
  it('passes the lease and read-only request to the runtime', async () => {
    const createMcpServer = mock(() => ({
      connect: async () => undefined,
    }))
    const runtime = {
      hasLease: (token: string) => token === 'lease-1',
      createMcpServer,
    }
    const app = createMcpRoutes({
      runtime: runtime as never,
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

  it('rejects an expired lease before constructing an MCP server', async () => {
    const createMcpServer = mock(() => ({
      connect: async () => undefined,
    }))
    const app = createMcpRoutes({
      runtime: {
        hasLease: () => false,
        createMcpServer,
      } as never,
      createMcpTransport: (() => new FakeTransport()) as never,
    })

    const response = await post(app, '/', {
      [BROWSEROS_TOOL_LEASE_HEADER]: 'expired',
    })

    expect(response.status).toBe(401)
    expect(createMcpServer).not.toHaveBeenCalled()
  })
})
