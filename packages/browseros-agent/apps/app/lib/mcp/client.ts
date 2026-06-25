/** @public */
export interface McpTool {
  name: string
  description?: string
}

const JSONRPC_VERSION = '2.0'
const MCP_PROTOCOL_VERSION = '2025-11-25'
const MCP_CLIENT_INFO = {
  name: 'browseros-settings',
  version: '1.0.0',
} as const

interface JsonRpcError {
  code: number
  message: string
  data?: unknown
}

interface JsonRpcRequest {
  jsonrpc: typeof JSONRPC_VERSION
  id?: number
  method: string
  params?: unknown
}

interface JsonRpcResponse<T> {
  jsonrpc: typeof JSONRPC_VERSION
  id: number
  result?: T
  error?: JsonRpcError
}

interface McpRequestContext {
  protocolVersion?: string
  sessionId?: string
}

interface InitializeResult {
  protocolVersion?: unknown
}

interface ListToolsResult {
  nextCursor?: unknown
  tools?: unknown
}

function buildMcpHeaders(
  context: McpRequestContext,
  accept: string,
): Record<string, string> {
  const headers: Record<string, string> = {
    accept,
  }

  if (context.sessionId) {
    headers['mcp-session-id'] = context.sessionId
  }
  if (context.protocolVersion) {
    headers['mcp-protocol-version'] = context.protocolVersion
  }

  return headers
}

async function readErrorMessage(response: Response): Promise<string> {
  const text = await response.text().catch(() => '')
  return text || response.statusText || `HTTP ${response.status}`
}

async function postJsonRpc<T>(
  serverUrl: string,
  message: JsonRpcRequest,
  context: McpRequestContext,
): Promise<{ result: T; sessionId?: string }> {
  const response = await fetch(serverUrl, {
    method: 'POST',
    headers: {
      ...buildMcpHeaders(context, 'application/json, text/event-stream'),
      'content-type': 'application/json',
    },
    body: JSON.stringify(message),
  })
  const sessionId = response.headers.get('mcp-session-id') ?? context.sessionId

  if (!response.ok) {
    throw new Error(`MCP request failed: ${await readErrorMessage(response)}`)
  }

  const contentType = response.headers.get('content-type') ?? ''
  if (!contentType.includes('application/json')) {
    throw new Error(
      `MCP request returned unsupported content type: ${contentType}`,
    )
  }

  const body = (await response.json()) as JsonRpcResponse<T>
  if (body.error) {
    throw new Error(`MCP request failed: ${body.error.message}`)
  }
  if (!('result' in body)) {
    throw new Error('MCP request returned no result')
  }

  return { result: body.result as T, sessionId }
}

async function postJsonRpcNotification(
  serverUrl: string,
  message: JsonRpcRequest,
  context: McpRequestContext,
): Promise<void> {
  const response = await fetch(serverUrl, {
    method: 'POST',
    headers: {
      ...buildMcpHeaders(context, 'application/json, text/event-stream'),
      'content-type': 'application/json',
    },
    body: JSON.stringify(message),
  })

  if (!response.ok) {
    throw new Error(
      `MCP notification failed: ${await readErrorMessage(response)}`,
    )
  }

  await response.body?.cancel()
}

async function openOptionalSseStream(
  serverUrl: string,
  context: McpRequestContext,
): Promise<void> {
  const response = await fetch(serverUrl, {
    method: 'GET',
    headers: buildMcpHeaders(context, 'text/event-stream'),
  })

  await response.body?.cancel()

  if (response.status === 405) {
    return
  }
  if (!response.ok) {
    throw new Error(
      `MCP SSE stream failed: ${await readErrorMessage(response)}`,
    )
  }
}

function normalizeTools(result: ListToolsResult): {
  nextCursor?: string
  tools: McpTool[]
} {
  if (!Array.isArray(result.tools)) {
    throw new Error('MCP tools/list returned invalid tools')
  }

  const tools = result.tools.map((tool, index) => {
    if (!tool || typeof tool !== 'object') {
      throw new Error(`MCP tools/list returned invalid tool at index ${index}`)
    }

    const { name, description } = tool as {
      description?: unknown
      name?: unknown
    }
    if (typeof name !== 'string') {
      throw new Error(`MCP tools/list returned unnamed tool at index ${index}`)
    }

    return {
      name,
      ...(typeof description === 'string' ? { description } : {}),
    }
  })

  return {
    tools,
    ...(typeof result.nextCursor === 'string'
      ? { nextCursor: result.nextCursor }
      : {}),
  }
}

/**
 * Fetches available tools from the BrowserOS MCP server without importing the
 * MCP SDK runtime, which can generate validators with `new Function` in tests
 * and browser extension contexts.
 * @public
 */
export async function fetchMcpTools(serverUrl: string): Promise<McpTool[]> {
  let nextId = 1
  const initialize = await postJsonRpc<InitializeResult>(
    serverUrl,
    {
      jsonrpc: JSONRPC_VERSION,
      id: nextId,
      method: 'initialize',
      params: {
        protocolVersion: MCP_PROTOCOL_VERSION,
        capabilities: {},
        clientInfo: MCP_CLIENT_INFO,
      },
    },
    {},
  )

  const protocolVersion = initialize.result.protocolVersion
  if (typeof protocolVersion !== 'string') {
    throw new Error('MCP initialize returned no protocol version')
  }

  const context: McpRequestContext = {
    protocolVersion,
    sessionId: initialize.sessionId,
  }

  await postJsonRpcNotification(
    serverUrl,
    {
      jsonrpc: JSONRPC_VERSION,
      method: 'notifications/initialized',
    },
    context,
  )
  await openOptionalSseStream(serverUrl, context)

  const tools: McpTool[] = []
  let cursor: string | undefined

  do {
    nextId += 1
    const response = await postJsonRpc<ListToolsResult>(
      serverUrl,
      {
        jsonrpc: JSONRPC_VERSION,
        id: nextId,
        method: 'tools/list',
        ...(cursor ? { params: { cursor } } : {}),
      },
      context,
    )
    const page = normalizeTools(response.result)

    tools.push(...page.tools)
    cursor = page.nextCursor
  } while (cursor)

  return tools
}
