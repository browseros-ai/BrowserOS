/**
 * Minimal streamable-HTTP MCP client over raw fetch. The suite drives
 * the Rust server's `/mcp` with this instead of the SDK client so it
 * controls the wire exactly: no Origin/Sec-Fetch-Site headers (the
 * the server rejects browser-shaped requests), explicit `mcp-session-id`
 * handling, and raw access for the transport-hygiene cases.
 *
 * rmcp answers POSTs with SSE streams by default, so the response reader also
 * accepts a plain JSON body when the transport selects that representation.
 */

export interface McpContent {
  type: string
  text?: string
  data?: string
  mimeType?: string
}

export interface McpToolResult {
  content: McpContent[]
  structuredContent?: Record<string, unknown>
  isError?: boolean
}

export interface McpToolInfo {
  name: string
  description?: string
  inputSchema?: Record<string, unknown>
}

interface JsonRpcResponse {
  jsonrpc: '2.0'
  id?: number | string | null
  result?: unknown
  error?: { code: number; message: string; data?: unknown }
}

export class McpRequestError extends Error {
  constructor(
    readonly code: number,
    message: string,
    readonly data?: unknown,
  ) {
    super(message)
  }
}

const PROTOCOL_VERSION = '2025-03-26'
const REQUEST_TIMEOUT_MS = 120_000

async function readSseResponse(
  response: Response,
  requestId: number,
): Promise<JsonRpcResponse> {
  const body = response.body
  if (!body) throw new Error('SSE response had no body')
  const reader = body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  try {
    while (true) {
      const { done, value } = await reader.read()
      if (value) buffer += decoder.decode(value, { stream: true })
      // Frames are separated by a blank line; data may span chunk reads.
      let boundary = buffer.indexOf('\n\n')
      while (boundary !== -1) {
        const frame = buffer.slice(0, boundary)
        buffer = buffer.slice(boundary + 2)
        boundary = buffer.indexOf('\n\n')
        const data = frame
          .split('\n')
          .filter((line) => line.startsWith('data:'))
          .map((line) => line.slice(5).trimStart())
          .join('\n')
        if (!data) continue
        const message = JSON.parse(data) as JsonRpcResponse
        if (message.id === requestId) return message
      }
      if (done) break
    }
  } finally {
    await reader.cancel().catch(() => {})
  }
  throw new Error(
    `SSE stream ended without a response for request ${requestId}`,
  )
}

async function readRpcResponse(
  response: Response,
  requestId: number,
): Promise<JsonRpcResponse> {
  const contentType = response.headers.get('content-type') ?? ''
  if (contentType.includes('text/event-stream')) {
    return await readSseResponse(response, requestId)
  }
  return (await response.json()) as JsonRpcResponse
}

export class McpSession {
  #nextId = 1
  sessionId: string | undefined

  private constructor(
    readonly baseUrl: string,
    readonly clientName: string,
  ) {}

  static async connect(
    baseUrl: string,
    clientName = 'claw-contract',
  ): Promise<McpSession> {
    const session = new McpSession(baseUrl, clientName)
    await session.#initialize()
    return session
  }

  #headers(): Record<string, string> {
    const headers: Record<string, string> = {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
    }
    if (this.sessionId) headers['mcp-session-id'] = this.sessionId
    return headers
  }

  async #initialize(): Promise<void> {
    const id = this.#nextId
    this.#nextId += 1
    const response = await fetch(`${this.baseUrl}/mcp`, {
      method: 'POST',
      headers: this.#headers(),
      body: JSON.stringify({
        jsonrpc: '2.0',
        id,
        method: 'initialize',
        params: {
          protocolVersion: PROTOCOL_VERSION,
          capabilities: {},
          clientInfo: { name: this.clientName, version: '0.0.1' },
        },
      }),
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    })
    if (!response.ok) {
      throw new Error(
        `initialize failed: ${response.status} ${await response.text()}`,
      )
    }
    this.sessionId = response.headers.get('mcp-session-id') ?? undefined
    const message = await readRpcResponse(response, id)
    if (message.error) {
      throw new McpRequestError(
        message.error.code,
        message.error.message,
        message.error.data,
      )
    }
    await fetch(`${this.baseUrl}/mcp`, {
      method: 'POST',
      headers: this.#headers(),
      body: JSON.stringify({
        jsonrpc: '2.0',
        method: 'notifications/initialized',
      }),
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    })
  }

  async request(method: string, params: unknown): Promise<unknown> {
    const id = this.#nextId
    this.#nextId += 1
    const response = await fetch(`${this.baseUrl}/mcp`, {
      method: 'POST',
      headers: this.#headers(),
      body: JSON.stringify({ jsonrpc: '2.0', id, method, params }),
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    })
    if (!response.ok) {
      throw new Error(
        `${method} failed: ${response.status} ${await response.text()}`,
      )
    }
    const message = await readRpcResponse(response, id)
    if (message.error) {
      throw new McpRequestError(
        message.error.code,
        message.error.message,
        message.error.data,
      )
    }
    return message.result
  }

  async listTools(): Promise<McpToolInfo[]> {
    const result = (await this.request('tools/list', {})) as {
      tools: McpToolInfo[]
    }
    return result.tools
  }

  async callTool(
    name: string,
    args: Record<string, unknown> = {},
  ): Promise<McpToolResult> {
    const result = await this.request('tools/call', { name, arguments: args })
    return result as McpToolResult
  }

  /**
   * DELETE teardown ends the session and audits the close. Send an explicit
   * content type to keep the wire request consistent with other MCP calls.
   */
  async close(): Promise<Response> {
    return await fetch(`${this.baseUrl}/mcp`, {
      method: 'DELETE',
      headers: {
        'content-type': 'application/json',
        ...(this.sessionId ? { 'mcp-session-id': this.sessionId } : {}),
      },
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    })
  }
}

/** Concatenated text of every text block in a tool result. */
export function textOf(result: McpToolResult): string {
  return result.content
    .filter((item) => item.type === 'text')
    .map((item) => item.text ?? '')
    .join('\n')
}

export function imageOf(
  result: McpToolResult,
): { data: string; mimeType: string } | undefined {
  const image = result.content.find((item) => item.type === 'image')
  if (!image?.data || !image.mimeType) return undefined
  return { data: image.data, mimeType: image.mimeType }
}
