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

const TAB = {
  pageId: 1,
  targetId: 'target-1',
  tabId: 11,
  url: 'https://example.com',
  title: 'Example',
  isActive: true,
  isLoading: false,
  loadProgress: 1,
  isPinned: false,
}

function tabSession() {
  return { pages: { list: async () => [TAB] } }
}

const SESSION_META_KEY = 'com.browseros/session'

function sessionHandle(result: Record<string, unknown> | undefined) {
  const meta = result?._meta as Record<string, unknown> | undefined
  return meta?.[SESSION_META_KEY] as string | undefined
}

function textOf(result: Record<string, unknown> | undefined): string {
  const content =
    (result?.content as Array<{ type?: string; text?: string }>) ?? []
  return content
    .filter((c) => c.type === 'text' && typeof c.text === 'string')
    .map((c) => c.text as string)
    .join('\n')
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
      call.json.result as { structuredContent?: { value?: unknown } }
    )?.structuredContent
    // run output is page-derived; the structured value is fenced as untrusted.
    expect(typeof structured?.value).toBe('string')
    expect(structured?.value).toContain('UNTRUSTED_PAGE_CONTENT')
    expect(structured?.value).toContain('42')
  })

  // Regression for #2651: for a lease-less external client the server mints a
  // session handle. It must ride in `_meta`, never replace a schemaless tool's
  // result in `structuredContent`, and never collide with run's output schema.
  it('modern: schemaless tool returns its result in content with the handle in _meta', async () => {
    const app = route(tabSession())

    const call = await post(
      app,
      {
        jsonrpc: '2.0',
        id: 1,
        method: 'tools/call',
        params: {
          name: 'tabs',
          arguments: { action: 'list' },
          _meta: CLIENT_META,
        },
      },
      {
        'MCP-Protocol-Version': '2026-07-28',
        'Mcp-Method': 'tools/call',
        'Mcp-Name': 'tabs',
      },
    )

    expect(call.status).toBe(200)
    expect(call.json.error).toBeUndefined()
    const result = call.json.result as Record<string, unknown>
    // The real tab list reaches the client through content, not replaced.
    expect(textOf(result)).toContain('https://example.com')
    // tabs declares no output schema and the route leaves structured content
    // off, so nothing (least of all a session-only object) rides it.
    expect(result.structuredContent).toBeUndefined()
    // The handle is delivered out-of-band in _meta.
    expect(typeof sessionHandle(result)).toBe('string')
    expect(sessionHandle(result)).toHaveLength(36)
  })

  it('legacy 2025-11-25: schemaless tool returns its result in content with the handle in _meta', async () => {
    const app = route(tabSession())

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
    expect(
      (init.json.result as { protocolVersion?: string })?.protocolVersion,
    ).toBe('2025-11-25')

    const call = await post(app, {
      jsonrpc: '2.0',
      id: 2,
      method: 'tools/call',
      params: { name: 'tabs', arguments: { action: 'list' } },
    })

    expect(call.status).toBe(200)
    expect(call.json.error).toBeUndefined()
    const result = call.json.result as Record<string, unknown>
    expect(textOf(result)).toContain('https://example.com')
    expect(result.structuredContent).toBeUndefined()
    expect(typeof sessionHandle(result)).toBe('string')
  })

  it('modern: run keeps a schema-valid structuredContent free of the handle, which rides in _meta', async () => {
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

    // No -32600: the handle no longer collides with the RunOutput schema.
    expect(call.status).toBe(200)
    expect(call.json.error).toBeUndefined()
    const result = call.json.result as Record<string, unknown>
    const structured = result.structuredContent as Record<string, unknown>
    expect(structured).toBeDefined()
    expect(structured.session).toBeUndefined()
    expect(typeof sessionHandle(result)).toBe('string')
  })
})
