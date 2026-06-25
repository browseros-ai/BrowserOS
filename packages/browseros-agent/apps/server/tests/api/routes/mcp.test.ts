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
  executionDir: string | undefined
  remoteAgentHarness: { outputFileAccess?: unknown } | undefined
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
    executionDir?: string
    remoteAgentHarness?: { outputFileAccess?: unknown }
  }) => {
    serverCreations.push({
      executionDir: deps.executionDir,
      remoteAgentHarness: deps.remoteAgentHarness,
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
    executionDir: '/tmp/browseros-execution',
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
        executionDir: '/tmp/browseros-execution',
        remoteAgentHarness: undefined,
        proxyStatus: { state: 'connecting' },
        selectedServerNames: [],
      },
      {
        executionDir: '/tmp/browseros-execution',
        remoteAgentHarness: undefined,
        proxyStatus: { state: 'ready', toolCount: 3 },
        selectedServerNames: ['Slack', 'Google Docs'],
      },
    ])
    expect(transportInstances).toHaveLength(2)
    expect(connectCalls).toEqual(transportInstances)
  })

  it('sets the remote agent harness context only for the remote harness source', async () => {
    const app = createTestMcpRoutes()

    const defaultResponse = await postMcp(app)
    const remoteHarnessResponse = await postMcp(
      app,
      {},
      '/?source=remote-agent-harness',
    )

    expect(defaultResponse.status).toBe(200)
    expect(remoteHarnessResponse.status).toBe(200)
    expect(serverCreations).toEqual([
      {
        executionDir: '/tmp/browseros-execution',
        remoteAgentHarness: undefined,
        proxyStatus: null,
        selectedServerNames: [],
      },
      {
        executionDir: '/tmp/browseros-execution',
        remoteAgentHarness: { outputFileAccess: expect.any(Object) },
        proxyStatus: null,
        selectedServerNames: [],
      },
    ])
  })

  it('keeps remote agent harness context stable by source', async () => {
    const app = createTestMcpRoutes()

    await postMcp(app, {}, '/?source=remote-agent-harness')
    await postMcp(app, {}, '/?source=remote-agent-harness')

    expect(serverCreations[0].remoteAgentHarness).toBe(
      serverCreations[1].remoteAgentHarness,
    )
  })
})
