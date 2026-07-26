import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { createServer, type IncomingMessage, type Server } from 'node:http'
import type { AddressInfo } from 'node:net'
import { fetchMcpTools } from './client'

const SCHEMA_COMPILATION_STACK_RE =
  /node_modules[/\\](?:zod|@modelcontextprotocol[/\\]sdk)[/\\]/

function readBody(req: IncomingMessage): Promise<string> {
  const chunks: Uint8Array[] = []

  return new Promise((resolve, reject) => {
    req.on('data', (chunk) => {
      chunks.push(typeof chunk === 'string' ? Buffer.from(chunk) : chunk)
    })
    req.on('end', () => {
      resolve(Buffer.concat(chunks).toString('utf8'))
    })
    req.on('error', reject)
  })
}

function listen(server: Server): Promise<void> {
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', resolve)
  })
}

function close(server: Server): Promise<void> {
  return new Promise((resolve, reject) => {
    server.close((error) => {
      if (error) {
        reject(error)
        return
      }

      resolve()
    })
  })
}

describe('fetchMcpTools', () => {
  let originalFunction: FunctionConstructor

  beforeEach(() => {
    originalFunction = globalThis.Function
  })

  afterEach(() => {
    globalThis.Function = originalFunction
  })

  it('lists tools without invoking Function-based schema compilation', async () => {
    const requests: string[] = []
    const functionCallStacks: string[] = []
    let getRequests = 0
    let initialized = false

    const server = createServer(async (req, res) => {
      if (req.method === 'GET') {
        getRequests += 1
        res.writeHead(405)
        res.end()
        return
      }

      const message = JSON.parse(await readBody(req)) as {
        id?: string | number
        method: string
      }
      requests.push(message.method)

      if (message.method === 'initialize') {
        res.writeHead(200, {
          'Content-Type': 'application/json',
          'mcp-session-id': 'test-session',
        })
        res.end(
          JSON.stringify({
            jsonrpc: '2.0',
            id: message.id,
            result: {
              protocolVersion: '2025-03-26',
              capabilities: {
                tools: {},
              },
              serverInfo: {
                name: 'test-server',
                version: '1.0.0',
              },
            },
          }),
        )
        return
      }

      if (message.method === 'notifications/initialized') {
        initialized = true
        res.writeHead(202)
        res.end()
        return
      }

      if (message.method === 'tools/list') {
        res.writeHead(200, {
          'Content-Type': 'application/json',
        })
        res.end(
          JSON.stringify({
            jsonrpc: '2.0',
            id: message.id,
            result: {
              tools: [
                {
                  name: 'browser_list_tabs',
                  description: 'List tabs',
                  inputSchema: {
                    type: 'object',
                  },
                  outputSchema: {
                    type: 'object',
                    properties: {
                      ok: {
                        type: 'boolean',
                      },
                    },
                  },
                },
              ],
            },
          }),
        )
        return
      }

      res.writeHead(500)
      res.end()
    })

    await listen(server)

    try {
      globalThis.Function = new Proxy(originalFunction, {
        apply(target, thisArg, argArray) {
          functionCallStacks.push(new Error().stack ?? '')
          return Reflect.apply(target, thisArg, argArray)
        },
        construct(target, argArray, newTarget) {
          functionCallStacks.push(new Error().stack ?? '')
          return Reflect.construct(target, argArray, newTarget)
        },
      }) as unknown as FunctionConstructor

      const { port } = server.address() as AddressInfo
      const tools = await fetchMcpTools(`http://127.0.0.1:${port}/mcp`)

      expect(tools).toEqual([
        {
          name: 'browser_list_tabs',
          description: 'List tabs',
        },
      ])
      expect(
        functionCallStacks.filter((stack) =>
          SCHEMA_COMPILATION_STACK_RE.test(stack),
        ),
      ).toEqual([])
      expect(getRequests).toBe(1)
      expect(initialized).toBe(true)
      expect(requests).toEqual([
        'initialize',
        'notifications/initialized',
        'tools/list',
      ])
    } finally {
      globalThis.Function = originalFunction
      await close(server)
    }
  })

  it('follows paginated tools/list responses', async () => {
    const cursors: Array<string | undefined> = []
    let listRequests = 0

    const server = createServer(async (req, res) => {
      if (req.method === 'GET') {
        res.writeHead(405)
        res.end()
        return
      }

      const message = JSON.parse(await readBody(req)) as {
        id?: string | number
        method: string
        params?: {
          cursor?: string
        }
      }

      if (message.method === 'initialize') {
        res.writeHead(200, {
          'Content-Type': 'application/json',
          'mcp-session-id': 'test-session',
        })
        res.end(
          JSON.stringify({
            jsonrpc: '2.0',
            id: message.id,
            result: {
              protocolVersion: '2025-03-26',
              capabilities: {
                tools: {},
              },
              serverInfo: {
                name: 'test-server',
                version: '1.0.0',
              },
            },
          }),
        )
        return
      }

      if (message.method === 'notifications/initialized') {
        res.writeHead(202)
        res.end()
        return
      }

      if (message.method === 'tools/list') {
        listRequests += 1
        cursors.push(message.params?.cursor)

        res.writeHead(200, {
          'Content-Type': 'application/json',
        })

        if (message.params?.cursor === 'cursor-1') {
          res.end(
            JSON.stringify({
              jsonrpc: '2.0',
              id: message.id,
              result: {
                tools: [
                  {
                    name: 'browser_get_page_content',
                    description: 'Get page content',
                    inputSchema: {
                      type: 'object',
                    },
                  },
                ],
              },
            }),
          )
          return
        }

        res.end(
          JSON.stringify({
            jsonrpc: '2.0',
            id: message.id,
            result: {
              nextCursor: 'cursor-1',
              tools: [
                {
                  name: 'browser_list_tabs',
                  description: 'List tabs',
                  inputSchema: {
                    type: 'object',
                  },
                },
              ],
            },
          }),
        )
        return
      }

      res.writeHead(500)
      res.end()
    })

    await listen(server)

    try {
      const { port } = server.address() as AddressInfo
      const tools = await fetchMcpTools(`http://127.0.0.1:${port}/mcp`)

      expect(tools).toEqual([
        {
          name: 'browser_list_tabs',
          description: 'List tabs',
        },
        {
          name: 'browser_get_page_content',
          description: 'Get page content',
        },
      ])
      expect(listRequests).toBe(2)
      expect(cursors).toEqual([undefined, 'cursor-1'])
    } finally {
      await close(server)
    }
  })
})
