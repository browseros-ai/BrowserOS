/**
 * Ungated unit tests for the raw MCP client against a scripted
 * streamable-HTTP stub: SSE parsing across chunk boundaries, session
 * header plumbing, hygiene (no Origin header), and error surfacing.
 */

import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { McpRequestError, McpSession, textOf } from './mcp-client'

interface RecordedRequest {
  method: string
  headers: Record<string, string>
  body?: Record<string, unknown>
}

const recorded: RecordedRequest[] = []
let server: ReturnType<typeof Bun.serve>

function sseResponse(
  message: Record<string, unknown>,
  headers: Record<string, string> = {},
): Response {
  const payload = JSON.stringify(message)
  // Split mid-payload so the client has to reassemble frames across reads.
  const first = `event: message\ndata: ${payload.slice(0, 12)}`
  const second = `${payload.slice(12)}\n\n`
  const stream = new ReadableStream({
    async start(controller) {
      controller.enqueue(new TextEncoder().encode(first))
      await Bun.sleep(5)
      controller.enqueue(new TextEncoder().encode(second))
      controller.close()
    },
  })
  return new Response(stream, {
    headers: { 'content-type': 'text/event-stream', ...headers },
  })
}

beforeAll(() => {
  server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: async (request) => {
      const headers: Record<string, string> = {}
      request.headers.forEach((value, key) => {
        headers[key] = value
      })
      const body =
        request.method === 'POST'
          ? ((await request.json()) as Record<string, unknown>)
          : undefined
      recorded.push({ method: request.method, headers, body })

      if (request.method === 'DELETE')
        return new Response(null, { status: 204 })
      if (body?.method === 'initialize') {
        return sseResponse(
          {
            jsonrpc: '2.0',
            id: body.id,
            result: { serverInfo: { name: 'stub' } },
          },
          { 'mcp-session-id': 'stub-session-1' },
        )
      }
      if (body?.method === 'notifications/initialized') {
        return new Response(null, { status: 202 })
      }
      if (body?.method === 'tools/list') {
        // Plain-JSON mode: the client must handle both response styles.
        return Response.json({
          jsonrpc: '2.0',
          id: body.id,
          result: { tools: [{ name: 'tabs' }, { name: 'snapshot' }] },
        })
      }
      if (body?.method === 'tools/call') {
        const params = body.params as { name: string }
        if (params.name === 'explode') {
          return Response.json({
            jsonrpc: '2.0',
            id: body.id,
            error: { code: -32602, message: 'invalid tool arguments' },
          })
        }
        return sseResponse({
          jsonrpc: '2.0',
          id: body.id,
          result: {
            content: [
              { type: 'text', text: `ran ${params.name}` },
              { type: 'text', text: 'second block' },
            ],
          },
        })
      }
      return new Response('unexpected', { status: 500 })
    },
  })
})

afterAll(() => {
  server.stop(true)
})

describe('mcp client', () => {
  test('initialize captures the session id and sends initialized', async () => {
    const session = await McpSession.connect(
      `http://127.0.0.1:${server.port}`,
      'stub-client',
    )
    expect(session.sessionId).toBe('stub-session-1')

    const initialize = recorded.find((r) => r.body?.method === 'initialize')
    if (!initialize) throw new Error('initialize request was not recorded')
    expect(initialize.headers.origin).toBeUndefined()
    expect(initialize.headers['sec-fetch-site']).toBeUndefined()
    expect(initialize.headers.accept).toContain('text/event-stream')
    const initializeParams = initialize.body?.params
    if (!initializeParams)
      throw new Error('initialize params were not recorded')
    expect(
      (initializeParams as { clientInfo: { name: string } }).clientInfo.name,
    ).toBe('stub-client')

    const initialized = recorded.find(
      (r) => r.body?.method === 'notifications/initialized',
    )
    expect(initialized?.headers['mcp-session-id']).toBe('stub-session-1')
  })

  test('parses SSE tool results split across chunks and JSON list results', async () => {
    const session = await McpSession.connect(`http://127.0.0.1:${server.port}`)
    const tools = await session.listTools()
    expect(tools.map((tool) => tool.name)).toEqual(['tabs', 'snapshot'])

    const result = await session.callTool('snapshot', { page: 1 })
    expect(textOf(result)).toBe('ran snapshot\nsecond block')
    const call = recorded.findLast((r) => r.body?.method === 'tools/call')
    expect(call?.headers['mcp-session-id']).toBe('stub-session-1')
  })

  test('JSON-RPC errors throw and DELETE closes with the session header', async () => {
    const session = await McpSession.connect(`http://127.0.0.1:${server.port}`)
    await expect(session.callTool('explode')).rejects.toThrow(McpRequestError)

    const response = await session.close()
    expect(response.status).toBe(204)
    const teardown = recorded.findLast((r) => r.method === 'DELETE')
    expect(teardown?.headers['mcp-session-id']).toBe('stub-session-1')
  })
})
