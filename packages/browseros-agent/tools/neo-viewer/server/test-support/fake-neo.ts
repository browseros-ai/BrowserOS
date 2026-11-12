/**
 * A stand-in for Neo's `/mcp` in tests: answers each request from a script (status, headers,
 * body chunks with pauses between them), and records what reached it, byte for byte, and
 * whether the caller hung up before the reply finished.
 */

export interface FakeReply {
  status?: number
  headers?: Record<string, string>
  /** Body pieces sent in order (text as UTF-8, bytes as they are); a number waits that many ms. */
  chunks: Array<string | Uint8Array | number>
}

export interface ReceivedRequest {
  method: string
  path: string
  headers: Headers
  body: string
  /** True once the caller closed the request before the whole reply was sent. */
  aborted: boolean
  /** When each body chunk was handed to the socket (ms since epoch). */
  sentAt: number[]
}

export interface FakeNeo {
  url: string
  requests: ReceivedRequest[]
  stop(): Promise<void>
}

export function startFakeNeo(
  respond: (request: ReceivedRequest) => FakeReply,
): FakeNeo {
  const requests: ReceivedRequest[] = []
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    idleTimeout: 0,
    async fetch(req) {
      const received: ReceivedRequest = {
        method: req.method,
        path: new URL(req.url).pathname,
        headers: req.headers,
        body: await req.text(),
        aborted: false,
        sentAt: [],
      }
      requests.push(received)
      req.signal.addEventListener('abort', () => {
        received.aborted = true
      })
      const reply = respond(received)
      const pieces = [...reply.chunks]
      const stream = new ReadableStream<Uint8Array>({
        async pull(controller) {
          for (;;) {
            const next = pieces.shift()
            if (next === undefined) {
              controller.close()
              return
            }
            if (typeof next === 'number') {
              await Bun.sleep(next)
              if (received.aborted) {
                controller.close()
                return
              }
              continue
            }
            received.sentAt.push(Date.now())
            controller.enqueue(
              typeof next === 'string' ? new TextEncoder().encode(next) : next,
            )
            return
          }
        },
      })
      return new Response(stream, {
        status: reply.status ?? 200,
        headers: reply.headers ?? { 'content-type': 'text/event-stream' },
      })
    },
  })
  return {
    url: `http://127.0.0.1:${server.port}/mcp`,
    requests,
    stop: () => server.stop(true),
  }
}

/** The SSE body Neo sends for one JSON-RPC message. */
export function sseEvent(message: unknown): string {
  return `data: ${JSON.stringify(message)}\n\n`
}

/** A `run` result as Neo builds it (`neo-server/src/mcp/run_tool.rs`). */
export function runResult(id: number | string, text: string, isError = false) {
  return {
    jsonrpc: '2.0',
    id,
    result: {
      content: [{ type: 'text', text }],
      ...(isError ? { isError: true } : {}),
      _meta: {
        'com.browseros.neo/run': {
          effects: 'none',
          id: '01a1cc18-ed14-78b3-a30d-2cb4ef81d9f4',
          status: isError ? 'error' : 'ok',
          unknown: [],
          tasks: [],
        },
      },
    },
  }
}
