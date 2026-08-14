import { beforeEach, describe, expect, it, mock } from 'bun:test'
import {
  createMcpRoutes,
  MANAGED_MCP_SERVERS_HEADER,
  parseManagedMcpServersHeader,
} from '../../../src/api/routes/mcp'
import type {
  ConnectorToolScope,
  KlavisProxyStatus,
} from '../../../src/api/services/klavis'

interface McpServerCreation {
  includeStructuredContent: boolean | undefined
  proxyStatus: KlavisProxyStatus | null
  selectedServerNames: readonly string[] | undefined
}

const serverCreations: McpServerCreation[] = []
const transportInstances: FakeTransport[] = []
const connectCalls: FakeTransport[] = []

class FakeTransport {
  constructor(readonly options: unknown) {
    transportInstances.push(this)
  }

  handleRequest = mock(async () => Response.json({ ok: true }))
}

const createMcpTransportSpy = mock((options: unknown) => {
  return new FakeTransport(options)
})

const createMcpServerSpy = mock(
  (deps: {
    klavis?: { getProxyStatus(): KlavisProxyStatus }
    connectorScope?: ConnectorToolScope
    includeStructuredContent?: boolean
  }) => {
    serverCreations.push({
      includeStructuredContent: deps.includeStructuredContent,
      proxyStatus: deps.klavis?.getProxyStatus() ?? null,
      selectedServerNames: deps.connectorScope?.selectedServerNames,
    })

    return {
      connect: mock(async (transport: FakeTransport) => {
        connectCalls.push(transport)
      }),
    }
  },
)

beforeEach(() => {
  serverCreations.length = 0
  transportInstances.length = 0
  connectCalls.length = 0
  createMcpServerSpy.mockClear()
  createMcpTransportSpy.mockClear()
})

function createTestMcpRoutes(
  overrides: Partial<Parameters<typeof createMcpRoutes>[0]> = {},
) {
  return createMcpRoutes({
    version: '0.0.0-test',
    browserSession: {} as never,
    createMcpServer: createMcpServerSpy as never,
    createMcpTransport: createMcpTransportSpy as never,
    ...overrides,
  })
}

async function postMcp(
  app: ReturnType<typeof createMcpRoutes>,
  headers: Record<string, string> = {},
  path = '/',
) {
  return app.request(path, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', ...headers },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/list',
    }),
  })
}

describe('parseManagedMcpServersHeader', () => {
  it('returns an empty scope for missing or empty headers', () => {
    expect(parseManagedMcpServersHeader(undefined)).toEqual([])
    expect(parseManagedMcpServersHeader('')).toEqual([])
  })

  it('parses comma-separated encoded connector names', () => {
    expect(parseManagedMcpServersHeader('Slack,Google%20Docs,Linear')).toEqual([
      'Slack',
      'Google Docs',
      'Linear',
    ])
  })

  it('degrades malformed encoded values to an empty scope', () => {
    expect(parseManagedMcpServersHeader('Slack,%E0%A4%A')).toEqual([])
  })
})

describe('createMcpRoutes', () => {
  it('passes latest Klavis status and selected connector scope per request', async () => {
    let status: KlavisProxyStatus = { state: 'connecting' }
    const klavis = {
      getProxyStatus: () => status,
    }
    const app = createTestMcpRoutes({
      klavis: klavis as never,
    })

    const first = await postMcp(app)

    status = { state: 'ready', toolCount: 3 }
    const second = await postMcp(app, {
      [MANAGED_MCP_SERVERS_HEADER]: 'Slack,Google%20Docs',
    })

    expect(first.status).toBe(200)
    expect(second.status).toBe(200)
    expect(serverCreations).toEqual([
      {
        includeStructuredContent: false,
        proxyStatus: { state: 'connecting' },
        selectedServerNames: [],
      },
      {
        includeStructuredContent: false,
        proxyStatus: { state: 'ready', toolCount: 3 },
        selectedServerNames: ['Slack', 'Google Docs'],
      },
    ])
    expect(transportInstances).toHaveLength(2)
    expect(connectCalls).toEqual(transportInstances)
  })

  it('opts into browser structured content only for structured=1', async () => {
    const app = createTestMcpRoutes()

    await postMcp(app)
    await postMcp(app, {}, '/?structured=1')
    await postMcp(app, {}, '/?structured=true')

    expect(
      serverCreations.map((creation) => creation.includeStructuredContent),
    ).toEqual([false, true, false])
  })

  it('returns the transport response verbatim, including its error status', async () => {
    const app = createTestMcpRoutes({
      createMcpTransport: (() => ({
        handleRequest: async () =>
          new Response('Not Acceptable', { status: 406 }),
      })) as never,
    })

    const res = await postMcp(app)

    expect(res.status).toBe(406)
  })

  it('returns 500 with a JSON-RPC internal error only for unexpected errors', async () => {
    const app = createTestMcpRoutes({
      createMcpTransport: (() => ({
        handleRequest: async () => {
          throw new Error('boom')
        },
      })) as never,
    })

    const res = await postMcp(app)
    const body = (await res.json()) as { error: { code: number } }

    expect(res.status).toBe(500)
    expect(body.error.code).toBe(-32603)
  })

  it('rejects browser-originated requests carrying Sec-Fetch-Site', async () => {
    const app = createTestMcpRoutes()

    const blocked = await postMcp(app, { 'Sec-Fetch-Site': 'cross-site' })
    const allowed = await postMcp(app)

    expect(blocked.status).toBe(403)
    const body = (await blocked.json()) as { error: { code: string } }
    expect(body.error.code).toBe('FORBIDDEN_BROWSER_REQUEST')
    expect(allowed.status).toBe(200)
  })
})
