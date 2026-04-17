#!/usr/bin/env bun
/**
 * BrowserOS MCP Server — Model Context Protocol adapter
 *
 * เปิดเผย BrowserOS headless server เป็น MCP tools
 * ให้ AI agent (เช่น OpenClaw) เรียกใช้ browser automation ได้
 *
 * รองรับ transport:
 *   - stdio (สำหรับ local / CLI)
 *   - SSE (สำหรับ remote / HTTP)
 *
 * วิธีใช้:
 *   bun run mcp-server.ts                    # stdio mode (default)
 *   MCP_TRANSPORT=sse MCP_PORT=3200 bun run mcp-server.ts  # SSE mode
 */

import { createInterface } from 'readline'
import { createServer } from 'http'
import { randomUUID } from 'crypto'
import { ALL_MCP_TOOLS, type McpToolHandler } from './mcp-tools'

// ─── การตั้งค่า ───────────────────────────────────────────
const HEADLESS_URL = process.env.HEADLESS_SERVER_URL || 'http://127.0.0.1:3100'
const HEADLESS_API_KEY = process.env.HEADLESS_API_KEY || ''
const MCP_TRANSPORT = process.env.MCP_TRANSPORT || 'stdio'
const MCP_PORT = Number(process.env.MCP_PORT || 3200)
const MCP_HOST = process.env.MCP_HOST || '0.0.0.0'

// ─── Session tracking (ตาม MCP client) ────────────────────
// แต่ละ MCP client จะมี browser session ของตัวเอง
const clientSessions = new Map<string, string>()

function getSessionId(clientId: string): string | undefined {
  return clientSessions.get(clientId)
}

async function setSessionId(clientId: string, sessionId: string): Promise<void> {
  clientSessions.set(clientId, sessionId)
}

// ─── HTTP helper — เรียก headless server API ──────────────
async function callHeadlessAPI(path: string, body?: Record<string, unknown>): Promise<any> {
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  }
  if (HEADLESS_API_KEY) {
    headers['Authorization'] = `Bearer ${HEADLESS_API_KEY}`
  }

  const url = `${HEADLESS_URL}${path}`
  const resp = await fetch(url, {
    method: body ? 'POST' : 'GET',
    headers,
    body: body ? JSON.stringify(body) : undefined,
  })

  const data = await resp.json()
  if (!resp.ok) {
    throw new Error(data.error || `HTTP ${resp.status}`)
  }
  return data
}

// ─── MCP Protocol types ───────────────────────────────────
interface JsonRpcRequest {
  jsonrpc: '2.0'
  id?: string | number
  method: string
  params?: Record<string, unknown>
}

interface JsonRpcResponse {
  jsonrpc: '2.0'
  id: string | number | null
  result?: unknown
  error?: {
    code: number
    message: string
    data?: unknown
  }
}

// ─── MCP Protocol handler ─────────────────────────────────
async function handleMcpRequest(
  request: JsonRpcRequest,
  clientId: string,
): Promise<JsonRpcResponse> {
  const { id, method, params = {} } = request

  try {
    switch (method) {
      // === MCP lifecycle ===
      case 'initialize': {
        return {
          jsonrpc: '2.0',
          id: id ?? null,
          result: {
            protocolVersion: '2024-11-05',
            capabilities: {
              tools: { listChanged: false },
            },
            serverInfo: {
              name: 'browseros-mcp',
              version: '1.0.0',
            },
          },
        }
      }

      case 'notifications/initialized': {
        // client ยืนยัน initialization เสร็จ — ไม่ต้องตอบ
        return { jsonrpc: '2.0', id: id ?? null, result: {} }
      }

      case 'ping': {
        return { jsonrpc: '2.0', id: id ?? null, result: {} }
      }

      // === Tools ===
      case 'tools/list': {
        return {
          jsonrpc: '2.0',
          id: id ?? null,
          result: {
            tools: ALL_MCP_TOOLS.map((t) => ({
              name: t.name,
              description: t.description,
              inputSchema: t.inputSchema,
            })),
          },
        }
      }

      case 'tools/call': {
        const toolName = params.name as string
        const args = (params.arguments as Record<string, unknown>) || {}

        const tool = ALL_MCP_TOOLS.find((t) => t.name === toolName)
        if (!tool) {
          return {
            jsonrpc: '2.0',
            id: id ?? null,
            error: {
              code: -32601,
              message: `ไม่พบ tool: ${toolName}`,
            },
          }
        }

        // ดึง session id ของ client (ถ้ามี)
        const sessionId = getSessionId(clientId)
        const handler = tool.handler as McpToolHandler

        const result = await handler(args, {
          sessionId,
          callAPI: callHeadlessAPI,
          setSessionId: (sid: string) => setSessionId(clientId, sid),
        })

        return {
          jsonrpc: '2.0',
          id: id ?? null,
          result: {
            content: [
              {
                type: 'text',
                text: typeof result === 'string' ? result : JSON.stringify(result, null, 2),
              },
            ],
          },
        }
      }

      default:
        return {
          jsonrpc: '2.0',
          id: id ?? null,
          error: {
            code: -32601,
            message: `ไม่รู้จัก method: ${method}`,
          },
        }
    }
  } catch (err: any) {
    return {
      jsonrpc: '2.0',
      id: id ?? null,
      error: {
        code: -32000,
        message: err.message || 'เกิดข้อผิดพลาดภายใน server',
      },
    }
  }
}

// ─── stdio transport ──────────────────────────────────────
function startStdioServer(): void {
  const rl = createInterface({ input: process.stdin })
  const clientId = 'stdio-client'

  // ส่ง initialization event
  const send = (msg: JsonRpcResponse) => {
    process.stdout.write(JSON.stringify(msg) + '\n')
  }

  rl.on('line', async (line) => {
    if (!line.trim()) return
    try {
      const request = JSON.parse(line) as JsonRpcRequest
      const response = await handleMcpRequest(request, clientId)
      // ไม่ตอบ notification ที่ไม่มี id
      if (request.id !== undefined) {
        send(response)
      }
    } catch (err: any) {
      send({
        jsonrpc: '2.0',
        id: null,
        error: { code: -32700, message: `JSON parse error: ${err.message}` },
      })
    }
  })

  console.error('[MCP] stdio transport พร้อมทำงานแล้ว')
}

// ─── SSE transport ────────────────────────────────────────
function startSSEServer(): void {
  const clients = new Map<string, { res: any; clientId: string }>()

  const server = createServer((req, res) => {
    // CORS
    res.setHeader('Access-Control-Allow-Origin', '*')
    res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS')
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type')

    if (req.method === 'OPTIONS') {
      res.writeHead(200)
      res.end()
      return
    }

    const url = new URL(req.url || '/', `http://${req.headers.host}`)

    // SSE endpoint — client มาต่อที่นี่เพื่อรับ events
    if (url.pathname === '/sse' && req.method === 'GET') {
      const clientId = randomUUID()
      res.writeHead(200, {
        'Content-Type': 'text/event-stream',
        'Cache-Control': 'no-cache',
        Connection: 'keep-alive',
      })

      // ส่ง endpoint URL ให้ client
      res.write(`event: endpoint\ndata: /messages?clientId=${clientId}\n\n`)

      clients.set(clientId, { res, clientId })
      console.error(`[MCP SSE] ลูกค้าเชื่อมต่อ: ${clientId}`)

      req.on('close', () => {
        clients.delete(clientId)
        console.error(`[MCP SSE] ลูกค้าตัดการเชื่อมต่อ: ${clientId}`)
      })
      return
    }

    // Message endpoint — client ส่ง JSON-RPC มาที่นี่
    if (url.pathname === '/messages' && req.method === 'POST') {
      const clientId = url.searchParams.get('clientId')
      if (!clientId) {
        res.writeHead(400, { 'Content-Type': 'application/json' })
        res.end(JSON.stringify({ error: 'ต้องระบุ clientId' }))
        return
      }

      let body = ''
      req.on('data', (chunk) => { body += chunk })
      req.on('end', async () => {
        try {
          const request = JSON.parse(body) as JsonRpcRequest
          const response = await handleMcpRequest(request, clientId)

          res.writeHead(200, { 'Content-Type': 'application/json' })
          res.end(JSON.stringify(response))

          // ส่ง SSE event ไปยัง client ด้วย
          const client = clients.get(clientId)
          if (client && request.id !== undefined) {
            client.res.write(`event: message\ndata: ${JSON.stringify(response)}\n\n`)
          }
        } catch (err: any) {
          res.writeHead(400, { 'Content-Type': 'application/json' })
          res.end(JSON.stringify({ error: err.message }))
        }
      })
      return
    }

    res.writeHead(404)
    res.end('ไม่พบ endpoint')
  })

  server.listen(MCP_PORT, MCP_HOST, () => {
    console.error(`[MCP SSE] server พร้อมที่ http://${MCP_HOST}:${MCP_PORT}/sse`)
  })
}

// ─── เริ่มทำงาน ──────────────────────────────────────────
console.error('═══════════════════════════════════════════════')
console.error('  🔌  BrowserOS MCP Server')
console.error('═══════════════════════════════════════════════')
console.error(`  Transport:     ${MCP_TRANSPORT}`)
console.error(`  Headless URL:  ${HEADLESS_URL}`)
console.error(`  Tools:         ${ALL_MCP_TOOLS.length} ตัว`)
console.error('═══════════════════════════════════════════════')

if (MCP_TRANSPORT === 'sse') {
  startSSEServer()
} else {
  startStdioServer()
}
