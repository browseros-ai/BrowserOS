/**
 * What the proxy reads from an agent's POST body, and the one edit it may make to it.
 *
 * A body is one JSON-RPC message or (protocol 2025-03-26) a batch of them. The proxy reads each
 * message's method, id and params to log the call; for a `run` call without a task id it adds
 * `_meta["com.browseros.neo/run"].taskId` by splicing text into the body as sent, so every other
 * byte reaches Neo unchanged.
 */

import type { ClientInfo } from '../../shared/api'
import {
  arrayItems,
  findMember,
  objectMembers,
  type Span,
  skipWhitespace,
} from './json-spans'

/** The `_meta` key Neo reads a host's ids from and answers a run's id under. */
export const RUN_META_KEY = 'com.browseros.neo/run'

/** Where a 2026-07-28 client puts its `clientInfo` on every request (SEP-2575). */
const CLIENT_INFO_META_KEY = 'io.modelcontextprotocol/clientInfo'

/** Neo's `TaskId::parse`: 1 to 128 bytes. */
const TASK_ID_MAX_BYTES = 128

export interface RequestMessage {
  /** The message's span in the body text. */
  span: Span
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
  try {
    text = utf8.decode(bytes)
    JSON.parse(text)
  } catch {
    return { text: null, messages: [] }
  }
  const start = skipWhitespace(text, 0)
  const spans =
    text[start] === '['
      ? arrayItems(text, start)
      : [{ start, end: text.length }]
  const messages: RequestMessage[] = []
  for (const span of spans ?? []) {
    const value = JSON.parse(text.slice(span.start, span.end)) as unknown
    if (!isObject(value)) continue
    messages.push({
      span,
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

/** A `tools/call` of the tool named `run`: the only call the proxy may stamp. */
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

/**
 * The text edit that gives `message` the task id, or null when the call must go unchanged:
 * it already names a task (or carries a `taskId` Neo will refuse), or its `_meta` is not an
 * object Neo would accept an added key in. `taskId: null` counts as none and is replaced.
 */
export function taskIdEdit(
  text: string,
  message: RequestMessage,
  taskId: string,
): Edit | null {
  const encoded = JSON.stringify(taskId)
  const top = objectMembers(text, message.span.start)
  const params = top && findMember(top, 'params')
  if (!params || text[params.value.start] !== '{') return null
  const paramMembers = objectMembers(text, params.value.start)
  if (!paramMembers) return null
  const meta = findMember(paramMembers, '_meta')
  if (!meta) {
    return insertFirst(
      text,
      params.value.start,
      `"_meta":{"${RUN_META_KEY}":{"taskId":${encoded}}}`,
    )
  }
  const metaMembers = objectMembers(text, meta.value.start)
  if (!metaMembers) return null
  const runMeta = findMember(metaMembers, RUN_META_KEY)
  if (!runMeta) {
    return insertFirst(
      text,
      meta.value.start,
      `"${RUN_META_KEY}":{"taskId":${encoded}}`,
    )
  }
  const runMembers = objectMembers(text, runMeta.value.start)
  if (!runMembers) return null
  const existing = findMember(runMembers, 'taskId')
  if (!existing)
    return insertFirst(text, runMeta.value.start, `"taskId":${encoded}`)
  if (text.slice(existing.value.start, existing.value.end) === 'null') {
    return { at: existing.value.start, remove: 4, insert: encoded }
  }
  return null
}

/** Replace `remove` characters at `at` with `insert`. */
export interface Edit {
  at: number
  remove: number
  insert: string
}

/** Put `member` first in the object opening at `objectStart`, before any existing member. */
function insertFirst(text: string, objectStart: number, member: string): Edit {
  const empty = text[skipWhitespace(text, objectStart + 1)] === '}'
  return {
    at: objectStart + 1,
    remove: 0,
    insert: empty ? member : `${member},`,
  }
}

/** Apply edits (at most one per message, so they never overlap), last first. */
export function applyEdits(text: string, edits: Edit[]): string {
  let out = text
  for (const edit of [...edits].sort((a, b) => b.at - a.at)) {
    out = out.slice(0, edit.at) + edit.insert + out.slice(edit.at + edit.remove)
  }
  return out
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
