import { beforeEach, describe, expect, it, mock } from 'bun:test'
import { createMcpRoutes } from '../../../src/api/routes/mcp'

interface McpServerCreation {
  leaseToken?: string
  requestedReadOnly?: boolean
  includeStructuredContent?: boolean
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

const createMcpServerSpy = mock((input: McpServerCreation) => {
  serverCreations.push(input)
  return {
    connect: mock(async (transport: FakeTransport) => {
      connectCalls.push(transport)
    }),
  }
})

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
    browserMcp: {
      validateLeaseToken: () => {},
      createMcpServer: createMcpServerSpy,
    } as never,
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

describe('createMcpRoutes', () => {
  it('opts into structured and read-only output only for exact query flags', async () => {
    const app = createTestMcpRoutes()

    await postMcp(app)
    await postMcp(app, {}, '/?structured=1&read_only=1')
    await postMcp(app, {}, '/?structured=true&read_only=true')

    expect(serverCreations).toEqual([
      {
        leaseToken: undefined,
        requestedReadOnly: false,
        includeStructuredContent: false,
      },
      {
        leaseToken: undefined,
        requestedReadOnly: true,
        includeStructuredContent: true,
      },
      {
        leaseToken: undefined,
        requestedReadOnly: false,
        includeStructuredContent: false,
      },
    ])
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
