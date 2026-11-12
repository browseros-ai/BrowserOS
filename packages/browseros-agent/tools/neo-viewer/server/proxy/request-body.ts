/**
 * What the proxy reads from an agent's POST body, for the log only: each JSON-RPC message's method,
 * id and params (a body is one message or, in protocol 2025-03-26, a batch). The body itself is
 * forwarded as sent; nothing here changes it.
 */

import type { ClientInfo } from '../../shared/api'

/** The `_meta` key Neo reads a host's ids from and answers a run's id under. */
export const RUN_META_KEY = 'com.browseros.neo/run'

/** Where a 2026-07-28 client puts its `clientInfo` on every request (SEP-2575). */
const CLIENT_INFO_META_KEY = 'io.modelcontextprotocol/clientInfo'

/** Neo's `TaskId::parse`: 1 to 128 bytes. */
const TASK_ID_MAX_BYTES = 128

export interface RequestMessage {
  method: string | null
  /** Absent for a notification. */
  id: string | number | null
  hasId: boolean
  params: Record<string, unknown> | null
}

export interface RequestBody {
  /** The body decoded as UTF-8, or null when it is not valid UTF-8 or not JSON. */
  text: string | null
  messages: RequestMessage[]
}

const utf8 = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true })

export function readRequestBody(bytes: Uint8Array): RequestBody {
  let text: string
  let parsed: unknown
  try {
    text = utf8.decode(bytes)
    parsed = JSON.parse(text)
  } catch {
    return { text: null, messages: [] }
  }
  const messages: RequestMessage[] = []
  for (const value of Array.isArray(parsed) ? parsed : [parsed]) {
    if (!isObject(value)) continue
    messages.push({
      method: typeof value.method === 'string' ? value.method : null,
      id:
        typeof value.id === 'string' || typeof value.id === 'number'
          ? value.id
          : null,
      hasId: 'id' in value,
      params: isObject(value.params) ? value.params : null,
    })
  }
  return { text, messages }
}

/** A `tools/call` of the tool named `run`. */
export function isRunCall(message: RequestMessage): boolean {
  return message.method === 'tools/call' && message.params?.name === 'run'
}

/** The agent's own task id: a valid `_meta["com.browseros.neo/run"].taskId` string. */
export function agentTaskId(message: RequestMessage): string | null {
  const runMeta = runMetaOf(message)
  const taskId = runMeta?.taskId
  if (typeof taskId !== 'string') return null
  const bytes = new TextEncoder().encode(taskId).length
  return bytes >= 1 && bytes <= TASK_ID_MAX_BYTES ? taskId : null
}

/** `clientInfo` a 2026-07-28 client sent in this request's `_meta`, or `initialize`'s. */
export function clientInfoOf(message: RequestMessage): ClientInfo | null {
  const params = message.params
  const meta = isObject(params?._meta) ? params._meta : null
  const raw =
    message.method === 'initialize'
      ? params?.clientInfo
      : meta?.[CLIENT_INFO_META_KEY]
  if (!isObject(raw) || typeof raw.name !== 'string' || raw.name === '')
    return null
  return {
    name: raw.name,
    ...(typeof raw.version === 'string' ? { version: raw.version } : {}),
    ...(typeof raw.title === 'string' ? { title: raw.title } : {}),
  }
}

function runMetaOf(message: RequestMessage): Record<string, unknown> | null {
  const meta = message.params?._meta
  if (!isObject(meta)) return null
  const runMeta = meta[RUN_META_KEY]
  return isObject(runMeta) ? runMeta : null
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
