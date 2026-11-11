/**
 * The viewer's MCP endpoint: streamable-HTTP MCP passed through to Neo, with each call logged.
 *
 *   agent ── POST /mcp ──▶ handleMcp ── fetch ──▶ Neo /mcp (the image's port 9010)
 *                            │  the body as sent, every byte: the viewer adds nothing
 *   agent ◀── each chunk ────┤◀── SSE or JSON reply
 *                            └─▶ ReplyTracker (after the chunk is forwarded) ─▶ RunStore
 *
 * Every method, path suffix and header goes through, so whatever MCP revision the agent speaks
 * (2026-07-28's `server/discover`, or `initialize`) reaches Neo as sent, and Neo behaves exactly
 * as without the viewer. Headers that belong to one hop (connection, framing) are the only ones
 * dropped. When the agent goes away mid-reply the upstream request is closed too: Neo's stateless
 * MCP stops a run when its client's stream is gone, so the proxy must not keep it alive.
 */

import type { ClientInfo } from '../../shared/api'
import type { RunStore } from '../log/run-store'
import {
  type Call,
  type Ending,
  ReplyTracker,
  resultOf,
  runEnd,
  toolsListSize,
} from './calls'
import {
  agentTaskId,
  clientInfoOf,
  type RequestBody,
  type RequestMessage,
  readRequestBody,
} from './request-body'

export interface McpProxyDeps {
  /** Neo's MCP endpoint, e.g. `http://127.0.0.1:9010/mcp`. */
  upstream: URL
  store: RunStore
  /**
   * `clientInfo` an older client sent once, in `initialize`, by its User-Agent: its later calls
   * carry none, and the list labels each run with its client.
   */
  clients: Map<string, ClientInfo>
  now: () => number
}

/** Hop-by-hop headers (RFC 9110 §7.6.1) and framing the next hop sets itself. */
const HOP_HEADERS = new Set([
  'connection',
  'keep-alive',
  'proxy-connection',
  'proxy-authenticate',
  'proxy-authorization',
  'te',
  'trailer',
  'transfer-encoding',
  'upgrade',
  'host',
  'content-length',
])

export async function handleMcp(
  req: Request,
  deps: McpProxyDeps,
): Promise<Response> {
  const bytes = req.body ? new Uint8Array(await req.arrayBuffer()) : null
  return new Exchange(req, bytes, deps).forward()
}

/** One HTTP request through the proxy and its reply, from arrival to the last logged line. */
class Exchange {
  private readonly sentAt: number
  private readonly userAgent: string | null
  private readonly body: RequestBody
  private readonly calls: Call[]
  private readonly tracker: ReplyTracker
  private readonly abort = new AbortController()
  private agentGone = false
  private finished = false
  private upstream: Response | null = null

  constructor(
    private readonly req: Request,
    private readonly bytes: Uint8Array | null,
    private readonly deps: McpProxyDeps,
  ) {
    this.sentAt = deps.now()
    this.userAgent = req.headers.get('user-agent')
    this.body = bytes ? readRequestBody(bytes) : { text: null, messages: [] }
    this.rememberClients()
    this.calls = this.body.messages.flatMap((message) => this.callFor(message))
    this.tracker = new ReplyTracker(this.calls)
    req.signal.addEventListener('abort', this.onAgentGone, { once: true })
  }

  async forward(): Promise<Response> {
    try {
      this.upstream = await fetch(
        upstreamUrl(this.req.url, this.deps.upstream),
        {
          method: this.req.method,
          headers: withoutHopHeaders(this.req.headers),
          body: this.bytes,
          signal: this.abort.signal,
          redirect: 'manual',
          // Bun decodes gzip by default; the agent must get Neo's bytes as they were sent.
          decompress: false,
        } as RequestInit,
      )
    } catch (error) {
      this.finish({ kind: 'failed', error: String(error) })
      if (this.agentGone) return new Response(null, { status: 499 })
      return new Response(
        `neo-viewer: Neo at ${this.deps.upstream.href} did not answer: ${String(error)}\n`,
        {
          status: 502,
          headers: { 'content-type': 'text/plain; charset=utf-8' },
        },
      )
    }
    return this.relay(this.upstream)
  }

  private readonly onAgentGone = () => {
    this.agentGone = true
    this.abort.abort()
  }

  private rememberClients(): void {
    if (!this.userAgent) return
    for (const message of this.body.messages) {
      const client =
        message.method === 'initialize' ? clientInfoOf(message) : null
      if (client) this.deps.clients.set(this.userAgent, client)
    }
  }

  /** The run's client: its own `clientInfo`, its `initialize`'s, or its User-Agent's product. */
  private clientFor(message: RequestMessage): ClientInfo | null {
    const sent = clientInfoOf(message)
    if (sent) return sent
    const remembered = this.userAgent
      ? this.deps.clients.get(this.userAgent)
      : undefined
    if (remembered) return remembered
    const product = this.userAgent?.split(/[\s/]/)[0]
    return product ? { name: product } : null
  }

  /** The logged call a message makes, if any: a tool call (a run, recorded now) or a `tools/list`. */
  private callFor(message: RequestMessage): Call[] {
    if (!message.hasId) return []
    if (message.method === 'tools/list')
      return [this.call(message, 'tools-list', null)]
    if (message.method !== 'tools/call') return []
    const params = message.params ?? {}
    const args = (isRecord(params.arguments) ? params.arguments : {}) as Record<
      string,
      unknown
    >
    const runId = this.deps.store.startRun({
      arrival: this.deps.store.nextArrival(),
      tool: typeof params.name === 'string' ? params.name : '',
      code: typeof args.code === 'string' ? args.code : '',
      timeoutMs: typeof args.timeout === 'number' ? args.timeout : null,
      client: this.clientFor(message),
      agentTaskId: agentTaskId(message),
      sentAt: new Date(this.sentAt).toISOString(),
      proxyMs: this.deps.now() - this.sentAt,
      request: { raw: this.body.text ?? '', jsonRpcId: message.id },
    })
    return [this.call(message, 'tool-call', runId)]
  }

  private call(
    message: RequestMessage,
    kind: Call['kind'],
    runId: string | null,
  ): Call {
    return {
      message,
      kind,
      idKey: JSON.stringify(message.id),
      reply: null,
      notifications: [],
      keepAlives: 0,
      runId,
    }
  }

  /** Sends Neo's reply on to the agent, chunk by chunk as it arrives, logging each after it is sent. */
  private async relay(answer: Response): Promise<Response> {
    this.tracker.headers(
      answer.headers.get('content-type'),
      answer.headers.get('content-encoding'),
    )
    // Every body is streamed, sized or not: a sized SSE reply can still arrive slowly. Bun frames
    // it chunked on this hop; the bytes are Neo's.
    const init = {
      status: answer.status,
      statusText: answer.statusText,
      headers: withoutHopHeaders(answer.headers),
    }
    if (!answer.body) {
      this.finish({ kind: 'ended' })
      return new Response(null, init)
    }
    const reader = answer.body.getReader()
    const stream = new ReadableStream<Uint8Array>({
      pull: async (controller) => {
        let chunk: Awaited<ReturnType<typeof reader.read>>
        try {
          chunk = await reader.read()
        } catch (error) {
          this.finish({ kind: 'failed', error: String(error) })
          controller.error(error)
          return
        }
        if (chunk.done) {
          this.finish({ kind: 'ended' })
          controller.close()
          return
        }
        controller.enqueue(chunk.value)
        this.tracker.push(chunk.value, this.deps.now())
      },
      cancel: (reason) => {
        this.agentGone = true
        this.finish({ kind: 'cancelled' })
        this.abort.abort()
        void reader.cancel(reason).catch(() => {})
      },
    })
    return new Response(stream, init)
  }

  private finish(ending: Ending): void {
    if (this.finished) return
    this.finished = true
    this.req.signal.removeEventListener('abort', this.onAgentGone)
    const endedAt = this.deps.now()
    this.tracker.end(endedAt)
    settle(
      this.calls,
      this.tracker,
      this.upstream,
      this.agentGone ? { kind: 'cancelled' } : ending,
      endedAt,
      this.deps.store,
    )
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** The upstream URL for a request to `/mcp[/suffix][?query]`. */
function upstreamUrl(requestUrl: string, upstream: URL): URL {
  const url = new URL(requestUrl)
  const target = new URL(upstream.href)
  const suffix = url.pathname.replace(/^\/mcp/, '')
  target.pathname = `${target.pathname.replace(/\/$/, '')}${suffix}`
  target.search = url.search
  return target
}

function withoutHopHeaders(headers: Headers): Headers {
  const out = new Headers()
  headers.forEach((value, name) => {
    if (!HOP_HEADERS.has(name.toLowerCase())) out.append(name, value)
  })
  return out
}

/** Records how each call ended, once the reply is over. */
function settle(
  calls: Call[],
  tracker: ReplyTracker,
  upstream: Response | null,
  ending: Ending,
  endedAt: number,
  store: RunStore,
): void {
  const raw = tracker.raw()
  for (const call of calls) {
    const result = resultOf(call.reply?.message)
    if (call.kind === 'tools-list') {
      const size = result ? toolsListSize(result) : null
      if (size) store.toolsListSeen(size, call.reply?.at ?? endedAt)
      continue
    }
    if (!call.runId) continue
    const coding = {
      contentType: tracker.contentTypeValue(),
      contentEncoding: tracker.contentEncodingValue(),
    }
    store.endRun(
      call.runId,
      runEnd(call, result, raw, coding, upstream, ending, endedAt),
    )
  }
}
