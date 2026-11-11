/**
 * The viewer's MCP endpoint: streamable-HTTP MCP passed through to Neo, with each call logged.
 *
 *   agent ── POST /mcp ──▶ handleMcp ── fetch ──▶ Neo /mcp (the image's port 9010)
 *                            │  adds _meta["com.browseros.neo/run"].taskId to a run call
 *                            │  that has none (the only change, in either direction)
 *   agent ◀── each chunk ────┤◀── SSE or JSON reply
 *                            └─▶ ReplyTracker (after the chunk is forwarded) ─▶ TaskStore
 *
 * Every method, path suffix and header goes through, so whatever MCP revision the agent speaks
 * (2026-07-28's `server/discover`, or `initialize`) reaches Neo as sent. Headers that belong to
 * one hop (connection, framing) are the only ones dropped. When the agent goes away mid-reply
 * the upstream request is closed too: Neo's stateless MCP stops a run when its client's stream
 * is gone, so the proxy must not keep it alive.
 */

import type { ClientInfo, JsonRpcError, TextSize } from '../../shared/api'
import { textSize } from '../../shared/api'
import type { StoredRunEnd } from '../log/task-log'
import type { TaskStore } from '../log/task-store'
import type { ClientNames } from '../tasks/client-names'
import type { Peer, ProcessFinder } from '../tasks/peer-process'
import type { Assignment, ProcessLookup, TaskRule } from '../tasks/task-rule'
import {
  JsonReader,
  type ReplyListener,
  type ReplyMessage,
  SseReader,
} from './reply-reader'
import {
  agentTaskId,
  applyEdits,
  clientInfoOf,
  type Edit,
  isRunCall,
  type RequestBody,
  type RequestMessage,
  readRequestBody,
  taskIdEdit,
} from './request-body'

export interface McpProxyDeps {
  /** Neo's MCP endpoint, e.g. `http://127.0.0.1:9010/mcp`. */
  upstream: URL
  store: TaskStore
  rule: TaskRule
  processes: ProcessFinder
  clients: ClientNames
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

/** One JSON-RPC request in the body whose reply the log wants: a tool call or `tools/list`. */
interface Call {
  message: RequestMessage
  kind: 'tool-call' | 'tools-list'
  idKey: string
  reply: ReplyMessage | null
  notifications: unknown[]
  keepAlives: number
  /** The task, once chosen (tool calls only). */
  assigned: Promise<Assignment> | null
  /** The run's id in the store, once recorded (tool calls only). */
  started: Promise<string> | null
}

/** How the reply ended, for calls it never answered. */
type Ending =
  | { kind: 'ended' }
  | { kind: 'cancelled' }
  | { kind: 'failed'; error: string }

export async function handleMcp(
  req: Request,
  peer: Peer | null,
  deps: McpProxyDeps,
): Promise<Response> {
  const bytes = req.body ? new Uint8Array(await req.arrayBuffer()) : null
  const exchange = new Exchange(req, peer, bytes, deps)
  const forwarded = await exchange.stampRuns()
  return exchange.forward(forwarded)
}

/** One HTTP request through the proxy and its reply, from arrival to the last logged line. */
class Exchange {
  private readonly sentAt: number
  private readonly connection: string | null
  private readonly userAgent: string | null
  private readonly body: RequestBody
  private readonly lookup: Promise<ProcessLookup>
  private readonly release: () => void
  private readonly calls: Call[]
  private readonly tracker: ReplyTracker
  private readonly abort = new AbortController()
  private agentGone = false
  private finished = false
  private upstream: Response | null = null

  constructor(
    private readonly req: Request,
    peer: Peer | null,
    private readonly bytes: Uint8Array | null,
    private readonly deps: McpProxyDeps,
  ) {
    this.sentAt = deps.now()
    this.connection = peer ? `${peer.address}:${peer.port}` : null
    this.userAgent = req.headers.get('user-agent')
    this.body = bytes ? readRequestBody(bytes) : { text: null, messages: [] }
    // Started at once, so a run that has to wait for it waits as little as possible; skipped for
    // requests no task needs (notifications, discovery).
    const needed = this.body.messages.some(
      (message) =>
        message.hasId &&
        ['tools/call', 'tools/list', 'initialize'].includes(
          message.method ?? '',
        ),
    )
    const found = needed
      ? deps.processes.find(peer)
      : {
          lookup: Promise.resolve<ProcessLookup>({
            found: false,
            reason: 'not looked up',
          }),
          release: () => {},
        }
    this.lookup = found.lookup
    this.release = found.release
    this.rememberClients()
    this.calls = this.body.messages.flatMap((message) => this.callFor(message))
    this.tracker = new ReplyTracker(this.calls)
    req.signal.addEventListener('abort', this.onAgentGone, { once: true })
  }

  /**
   * Gives each run without a task id its task's id, and returns the body to forward (null:
   * forward the bytes as sent). Only such a run waits for its task, since the stamp names it.
   */
  async stampRuns(): Promise<string | null> {
    const { text } = this.body
    const edits = new Map<Call, Edit>()
    for (const call of this.calls) {
      if (!call.assigned || text === null || !isRunCall(call.message)) continue
      if (agentTaskId(call.message) !== null) continue
      const assignment = await call.assigned
      const edit = taskIdEdit(text, call.message, assignment.taskId)
      if (edit) edits.set(call, edit)
    }
    const forwarded =
      text !== null && edits.size > 0
        ? applyEdits(text, [...edits.values()])
        : null
    this.recordStarts(edits, forwarded)
    return forwarded
  }

  async forward(forwarded: string | null): Promise<Response> {
    try {
      this.upstream = await fetch(
        upstreamUrl(this.req.url, this.deps.upstream),
        {
          method: this.req.method,
          headers: withoutHopHeaders(this.req.headers),
          body:
            forwarded !== null
              ? new TextEncoder().encode(forwarded)
              : this.bytes,
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
    for (const message of this.body.messages) {
      if (message.method !== 'initialize') continue
      const client = clientInfoOf(message)
      if (!client) continue
      const { clients } = this.deps
      clients.remember(client, {
        connection: this.connection,
        pid: null,
        userAgent: this.userAgent,
      })
      void this.lookup.then((process) => {
        if (process.found)
          clients.remember(client, {
            connection: null,
            pid: process.pid,
            userAgent: null,
          })
      })
    }
  }

  private clientFor(
    message: RequestMessage,
    process: ProcessLookup,
  ): ClientInfo {
    return this.deps.clients.resolve(clientInfoOf(message), {
      connection: this.connection,
      pid: process.found ? process.pid : null,
      userAgent: this.userAgent,
    })
  }

  /** The logged call a message makes, if any: a tool call (a run) or a `tools/list`. */
  private callFor(message: RequestMessage): Call[] {
    if (!message.hasId) return []
    const kind =
      message.method === 'tools/call'
        ? 'tool-call'
        : message.method === 'tools/list'
          ? 'tools-list'
          : null
    if (!kind) return []
    const assigned =
      kind === 'tool-call'
        ? this.lookup.then((process) => this.assign(message, process))
        : null
    return [
      {
        message,
        kind,
        idKey: JSON.stringify(message.id),
        reply: null,
        notifications: [],
        keepAlives: 0,
        assigned,
        started: null,
      },
    ]
  }

  private assign(message: RequestMessage, process: ProcessLookup): Assignment {
    const client = this.clientFor(message, process)
    const assignment = this.deps.rule.assign({
      agentTaskId: agentTaskId(message),
      client,
      process,
      at: this.sentAt,
    })
    if (assignment.created)
      this.deps.store.createTask(assignment, client, this.sentAt)
    return assignment
  }

  private recordStarts(edits: Map<Call, Edit>, forwarded: string | null): void {
    const forwardedAt = this.deps.now()
    for (const call of this.calls) {
      if (!call.assigned) continue
      const params = call.message.params ?? {}
      const args = (
        isRecord(params.arguments) ? params.arguments : {}
      ) as Record<string, unknown>
      const stamped = edits.has(call)
      call.started = call.assigned.then((assignment) =>
        this.deps.store.startRun(assignment.taskId, {
          tool: typeof params.name === 'string' ? params.name : '',
          code: typeof args.code === 'string' ? args.code : '',
          timeoutMs: typeof args.timeout === 'number' ? args.timeout : null,
          taskIdStamped: stamped,
          sentAt: new Date(this.sentAt).toISOString(),
          proxyMs: forwardedAt - this.sentAt,
          request: {
            raw: this.body.text ?? '',
            forwarded: stamped ? forwarded : null,
            jsonRpcId: call.message.id,
          },
        }),
      )
    }
  }

  /** Sends Neo's reply on to the agent, chunk by chunk as it arrives, logging each after it is sent. */
  private async relay(answer: Response): Promise<Response> {
    this.tracker.contentType(answer.headers.get('content-type'))
    const init = {
      status: answer.status,
      statusText: answer.statusText,
      headers: withoutHopHeaders(answer.headers),
    }
    if (!answer.body) {
      this.finish({ kind: 'ended' })
      return new Response(null, init)
    }
    if (answer.headers.has('content-length')) {
      // A short, sized body (a 202, a refusal): read it whole and keep its length.
      let whole: Uint8Array<ArrayBuffer>
      try {
        whole = new Uint8Array(await answer.arrayBuffer())
      } catch (error) {
        this.finish({ kind: 'failed', error: String(error) })
        return new Response(null, { status: this.agentGone ? 499 : 502 })
      }
      this.tracker.push(whole, this.deps.now())
      this.finish({ kind: 'ended' })
      return new Response(whole, init)
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
      {
        deps: this.deps,
        lookup: this.lookup,
        clientFor: (message, process) => this.clientFor(message, process),
      },
    )
    this.release()
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

/** Collects the reply's raw bytes and the messages in it, routing each to its call. */
class ReplyTracker implements ReplyListener {
  private readonly chunks: Uint8Array[] = []
  private reader: SseReader | JsonReader | null = null
  private type: string | null = null

  constructor(private readonly calls: Call[]) {}

  contentType(value: string | null): void {
    this.type = value
    const lower = value?.toLowerCase() ?? ''
    if (lower.startsWith('text/event-stream')) this.reader = new SseReader(this)
    else if (lower.includes('json')) this.reader = new JsonReader(this)
  }

  push(chunk: Uint8Array, at: number): void {
    this.chunks.push(chunk)
    this.reader?.push(chunk, at)
  }

  end(at: number): void {
    this.reader?.end(at)
    this.reader = null
  }

  /** The body as the agent got it, or null when no reply arrived. */
  raw(): string | null {
    return this.type === null
      ? null
      : new TextDecoder().decode(Buffer.concat(this.chunks))
  }

  contentTypeValue(): string | null {
    return this.type
  }

  message(reply: ReplyMessage): void {
    const message = reply.message
    if (isResponse(message)) {
      const key = JSON.stringify(message.id)
      const call = this.calls.find(
        (candidate) => candidate.idKey === key && !candidate.reply,
      )
      if (call) call.reply = reply
      return
    }
    for (const call of this.calls)
      if (!call.reply) call.notifications.push(message)
  }

  comment(): void {
    for (const call of this.calls) if (!call.reply) call.keepAlives++
  }
}

function isResponse(
  message: unknown,
): message is { id: unknown; result?: unknown; error?: unknown } {
  return (
    typeof message === 'object' &&
    message !== null &&
    'id' in message &&
    ('result' in message || 'error' in message)
  )
}

interface SettleContext {
  deps: McpProxyDeps
  lookup: Promise<ProcessLookup>
  clientFor: (message: RequestMessage, process: ProcessLookup) => ClientInfo
}

/** Records how each call ended, once the reply is over. */
function settle(
  calls: Call[],
  tracker: ReplyTracker,
  upstream: Response | null,
  ending: Ending,
  endedAt: number,
  { deps, lookup, clientFor }: SettleContext,
): void {
  const raw = tracker.raw()
  for (const call of calls) {
    const result = resultOf(call.reply?.message)
    if (call.kind === 'tools-list') {
      const size = result ? toolsListSize(result) : null
      if (!size) continue
      const at = call.reply?.at ?? endedAt
      void lookup.then((process) => {
        const agentKey = deps.rule.agentKey(
          clientFor(call.message, process),
          process,
        )
        deps.store.toolsListSeen(
          agentKey,
          deps.rule.taskFor(agentKey),
          size,
          at,
        )
      })
      continue
    }
    const end = runEnd(
      call,
      result,
      raw,
      tracker.contentTypeValue(),
      upstream,
      ending,
      endedAt,
    )
    if (!call.started || !call.assigned) continue
    const assigned = call.assigned
    void call.started.then(async (runId) => {
      deps.store.endRun(runId, end)
      const assignment = await assigned
      deps.rule.callEnded(
        assignment.key,
        assignment.taskId,
        Date.parse(end.answeredAt),
      )
    })
  }
}

function runEnd(
  call: Call,
  result: Record<string, unknown> | null,
  raw: string | null,
  contentType: string | null,
  upstream: Response | null,
  ending: Ending,
  endedAt: number,
): StoredRunEnd {
  const message = call.reply?.message as { error?: unknown } | undefined
  const rpcError =
    message && 'error' in message ? toRpcError(message.error) : null
  const content = Array.isArray(result?.content)
    ? (result.content as unknown[])
    : null
  const text = content
    ? content
        .filter((part): part is { type: 'text'; text: string } =>
          isTextPart(part),
        )
        .map((part) => part.text)
        .join('\n')
    : null
  const isError = result ? result.isError === true : null
  let status: StoredRunEnd['status']
  if (result) status = isError ? 'error' : 'ok'
  else if (rpcError) status = 'rpc-error'
  else status = ending.kind === 'cancelled' ? 'cancelled' : 'http-error'
  return {
    status,
    answeredAt: new Date(call.reply?.at ?? endedAt).toISOString(),
    reply: {
      httpStatus: upstream?.status ?? null,
      contentType,
      raw: raw ?? (ending.kind === 'failed' ? ending.error : null),
      text,
      structuredContent: result?.structuredContent ?? null,
      isError,
      meta: result?._meta ?? null,
      rpcError,
      notifications: call.notifications,
      keepAlives: call.keepAlives,
    },
  }
}

function resultOf(message: unknown): Record<string, unknown> | null {
  if (typeof message !== 'object' || message === null || !('result' in message))
    return null
  const result = (message as { result: unknown }).result
  return typeof result === 'object' && result !== null
    ? (result as Record<string, unknown>)
    : null
}

function isTextPart(part: unknown): boolean {
  return (
    typeof part === 'object' &&
    part !== null &&
    (part as { type?: unknown }).type === 'text' &&
    typeof (part as { text?: unknown }).text === 'string'
  )
}

function toRpcError(error: unknown): JsonRpcError {
  const value = (
    typeof error === 'object' && error !== null ? error : {}
  ) as Record<string, unknown>
  return {
    code: typeof value.code === 'number' ? value.code : 0,
    message:
      typeof value.message === 'string' ? value.message : JSON.stringify(error),
    ...('data' in value ? { data: value.data } : {}),
  }
}

/**
 * What a `tools/list` puts in the model's context: each tool's name, description and input
 * schema as JSON. Neo's one tool, `run`, is about 6,500 characters.
 */
function toolsListSize(result: Record<string, unknown>): TextSize | null {
  if (!Array.isArray(result.tools)) return null
  let text = ''
  for (const tool of result.tools as Array<Record<string, unknown>>) {
    text += `${String(tool.name ?? '')}${String(tool.description ?? '')}${JSON.stringify(tool.inputSchema ?? null)}`
  }
  return textSize(text)
}
