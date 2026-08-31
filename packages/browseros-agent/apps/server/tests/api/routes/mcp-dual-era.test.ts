import { describe, expect, it } from 'bun:test'
import { createMcpRoutes } from '../../../src/api/routes/mcp'
import { BrowserMcpModule } from '../../../src/api/services/mcp/browser-mcp-module'

const BASE = {
  'Content-Type': 'application/json',
  Accept: 'application/json, text/event-stream',
}
const CLIENT_META = {
  'io.modelcontextprotocol/protocolVersion': '2026-07-28',
  'io.modelcontextprotocol/clientInfo': { name: 'test', version: '1.0.0' },
  'io.modelcontextprotocol/clientCapabilities': {},
}

function route(browserSession: unknown = {}) {
  return createMcpRoutes({
    browserMcp: new BrowserMcpModule({
      version: '0.0.0-test',
      browserSession: browserSession as never,
      conversationRuns: { activeRun: () => undefined },
    }),
  })
}

async function post(
  app: ReturnType<typeof createMcpRoutes>,
  body: unknown,
  headers: Record<string, string> = {},
) {
  const res = await app.request('/', {
    method: 'POST',
    headers: { ...BASE, ...headers },
    body: JSON.stringify(body),
  })
  return {
    status: res.status,
    contentType: res.headers.get('content-type') ?? '',
    json: (await res.json()) as {
      result?: Record<string, unknown>
      error?: { code: number }
    },
  }
}

describe('mcp dual-era serving', () => {
  it('serves legacy clients over initialize, as JSON, negotiating 2025-11-25', async () => {
    const app = route()

    const init = await post(app, {
      jsonrpc: '2.0',
      id: 1,
      method: 'initialize',
      params: {
        protocolVersion: '2025-11-25',
        capabilities: {},
        clientInfo: { name: 't', version: '1' },
      },
    })

    expect(init.status).toBe(200)
    // The split keeps legacy on the enableJsonResponse transport, not SSE.
    expect(init.contentType).toContain('application/json')
    expect(
      (init.json.result as { protocolVersion?: string })?.protocolVersion,
    ).toBe('2025-11-25')

    const list = await post(app, {
      jsonrpc: '2.0',
      id: 2,
      method: 'tools/list',
    })
    expect(list.status).toBe(200)
    const tools = (list.json.result as { tools?: unknown[] })?.tools ?? []
    expect(tools).toHaveLength(17)
  })

  it('serves modern clients over server/discover, advertising 2026-07-28', async () => {
    const app = route()

    const discover = await post(
      app,
      {
        jsonrpc: '2.0',
        id: 1,
        method: 'server/discover',
        params: { _meta: CLIENT_META },
      },
      { 'MCP-Protocol-Version': '2026-07-28', 'Mcp-Method': 'server/discover' },
    )

    expect(discover.status).toBe(200)
    expect(discover.json.error).toBeUndefined()
    const supportedVersions = (
      discover.json.result as { supportedVersions?: string[] }
    )?.supportedVersions
    expect(supportedVersions).toContain('2026-07-28')
  })

  it('runs a modern tool call', async () => {
    const app = route()

    const call = await post(
      app,
      {
        jsonrpc: '2.0',
        id: 1,
        method: 'tools/call',
        params: {
          name: 'run',
          arguments: { code: 'return 42' },
          _meta: CLIENT_META,
        },
      },
      {
        'MCP-Protocol-Version': '2026-07-28',
        'Mcp-Method': 'tools/call',
        'Mcp-Name': 'run',
      },
    )

    expect(call.status).toBe(200)
    const structured = (
      call.json.result as { structuredContent?: { value?: number } }
    )?.structuredContent
    expect(structured?.value).toBe(42)
  })
})
