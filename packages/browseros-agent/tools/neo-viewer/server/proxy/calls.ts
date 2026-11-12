/**
 * One logged call inside an HTTP exchange (a tool call or a `tools/list`), what its reply said,
 * and the record the log keeps when it ends. Pure data handling: `mcp-proxy.ts` decides when.
 */

import { brotliDecompressSync, gunzipSync, inflateSync } from 'node:zlib'
import type { JsonRpcError, TextSize } from '../../shared/api'
import { textSize } from '../../shared/api'
import type { StoredRunEnd } from '../log/run-log'
import {
  JsonReader,
  type ReplyListener,
  type ReplyMessage,
  SseReader,
} from './reply-reader'
import type { RequestMessage } from './request-body'

/** Decodes a whole body by its `Content-Encoding`; null when the coding is unknown or broken. */
function decode(body: Buffer, encoding: string): Uint8Array | null {
  try {
    if (encoding === 'gzip' || encoding === 'x-gzip') return gunzipSync(body)
    if (encoding === 'deflate') return inflateSync(body)
    if (encoding === 'br') return brotliDecompressSync(body)
    if (encoding === 'zstd') return Bun.zstdDecompressSync(body)
  } catch {
    // Logged with no reply; the agent still got Neo's bytes.
  }
  return null
}

/** One JSON-RPC request in the body whose reply the log wants: a tool call or `tools/list`. */
export interface Call {
  message: RequestMessage
  kind: 'tool-call' | 'tools-list'
  idKey: string
  reply: ReplyMessage | null
  notifications: unknown[]
  keepAlives: number
  /** The run's id in the store (tool calls only). */
  runId: string | null
}

/** How the reply ended, for calls it never answered. */
export type Ending =
  | { kind: 'ended' }
  | { kind: 'cancelled' }
  | { kind: 'failed'; error: string }

/** Collects the reply's raw bytes and the messages in it, routing each to its call. */
export class ReplyTracker implements ReplyListener {
  private readonly chunks: Uint8Array[] = []
  private reader: SseReader | JsonReader | null = null
  private type: string | null = null
  private encoding: string | null = null
  private decoded: Uint8Array | null = null

  constructor(private readonly calls: Call[]) {}

  /**
   * The reply's content type picks the reader. An encoded body (gzip and the like) is passed on
   * as it came; the log decodes its own copy once the body ends.
   */
  headers(contentType: string | null, contentEncoding: string | null): void {
    this.type = contentType
    const encoding = contentEncoding?.trim().toLowerCase() ?? ''
    this.encoding = encoding === '' || encoding === 'identity' ? null : encoding
    const lower = contentType?.toLowerCase() ?? ''
    if (lower.startsWith('text/event-stream')) this.reader = new SseReader(this)
    else if (lower.includes('json')) this.reader = new JsonReader(this)
  }

  push(chunk: Uint8Array, at: number): void {
    this.chunks.push(chunk)
    if (!this.encoding) this.reader?.push(chunk, at)
  }

  end(at: number): void {
    if (this.encoding) {
      this.decoded = decode(Buffer.concat(this.chunks), this.encoding)
      if (this.decoded) this.reader?.push(this.decoded, at)
    }
    this.reader?.end(at)
    this.reader = null
  }

  /** The body as the agent read it (decoded when it was encoded), or null when no reply arrived. */
  raw(): string | null {
    if (this.type === null) return null
    const bytes = this.encoding ? this.decoded : Buffer.concat(this.chunks)
    return bytes ? new TextDecoder().decode(bytes) : null
  }

  contentTypeValue(): string | null {
    return this.type
  }

  contentEncodingValue(): string | null {
    return this.encoding
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

export function runEnd(
  call: Call,
  result: Record<string, unknown> | null,
  raw: string | null,
  coding: { contentType: string | null; contentEncoding: string | null },
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
      contentType: coding.contentType,
      contentEncoding: coding.contentEncoding,
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

export function resultOf(message: unknown): Record<string, unknown> | null {
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
export function toolsListSize(
  result: Record<string, unknown>,
): TextSize | null {
  if (!Array.isArray(result.tools)) return null
  let text = ''
  for (const tool of result.tools as Array<Record<string, unknown>>) {
    text += `${String(tool.name ?? '')}${String(tool.description ?? '')}${JSON.stringify(tool.inputSchema ?? null)}`
  }
  return textSize(text)
}
