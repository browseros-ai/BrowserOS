/** Strict wire oracle: keep framing/order/omission visible, normalize only named volatile fields. */
import { createRequire } from 'node:module'
import { resolve } from 'node:path'
import { readUiMessageStream } from '../../packages/server-api-client/src/stream'

type Json = Record<string, unknown>
const appRequire = createRequire(
  resolve(import.meta.dir, '../../apps/app/package.json'),
)
// Resolve through the app, not the harness: its installed SDK is the client's actual validator.
const { uiMessageChunkSchema, readUIMessageStream } = appRequire(
  'ai',
) as typeof import('../../apps/app/node_modules/ai')

export function parseSse(raw: string): Json[] {
  if (!raw.endsWith('data: [DONE]\n\n'))
    throw new Error('Missing final [DONE] frame')
  const frames = raw.split('\n\n')
  frames.pop()
  const chunks: Json[] = []
  for (const [index, frame] of frames.entries()) {
    if (frame === 'data: [DONE]') {
      if (index !== frames.length - 1)
        throw new Error('[DONE] before end of stream')
      continue
    }
    if (!frame.startsWith('data: '))
      throw new Error(`Invalid SSE framing at frame ${index}`)
    const chunk = JSON.parse(frame.slice(6)) as Json
    if (
      !chunk ||
      typeof chunk !== 'object' ||
      Array.isArray(chunk) ||
      frame !== `data: ${JSON.stringify(chunk)}`
    )
      throw new Error(`Noncanonical SSE JSON/framing at frame ${index}`)
    chunks.push(chunk)
  }
  return chunks
}

/** Identity maps preserve correlations, including a regression that references an unseen tool/text id. */
export class Ids {
  private maps = new Map<string, Map<string, string>>()
  id(kind: string, value: string): string {
    let map = this.maps.get(kind)
    if (!map) {
      map = new Map()
      this.maps.set(kind, map)
    }
    let result = map.get(value)
    if (!result) {
      result = `<${kind}:${map.size + 1}>`
      map.set(value, result)
    }
    return result
  }
}

export function normalizeSse(
  raw: string,
  options: { acp?: boolean } = {},
): string {
  const ids = new Ids()
  return (
    parseSse(raw)
      .filter(
        (chunk) =>
          !(
            options.acp &&
            chunk.type === 'tool-input-error' &&
            chunk.providerExecuted === true &&
            chunk.dynamic === true &&
            chunk.errorText === 'The ACP agent failed to respond.'
          ),
      )
      .map((chunk) => {
        if (chunk.type === 'start') delete chunk.messageId // F8: built-in TS omits the generated assistant id.
        if (
          typeof chunk.id === 'string' &&
          /^(text|reasoning)-/.test(String(chunk.type))
        )
          chunk.id = ids.id(String(chunk.type).split('-')[0], chunk.id)
        if (typeof chunk.toolCallId === 'string')
          chunk.toolCallId = ids.id('tool', chunk.toolCallId)
        return `data: ${JSON.stringify(chunk)}\n\n`
      })
      .join('') + 'data: [DONE]\n\n'
  )
}

export function diffBytes(a: string, b: string): string | null {
  const left = new TextEncoder().encode(a)
  const right = new TextEncoder().encode(b)
  let i = 0
  while (i < Math.min(left.length, right.length) && left[i] === right[i]) i++
  if (i === left.length && i === right.length) return null
  const decoder = new TextDecoder()
  return `First difference at byte ${i} (${left.length} vs ${right.length} bytes):\nA ${JSON.stringify(decoder.decode(left.slice(Math.max(0, i - 60), i + 160)))}\nB ${JSON.stringify(decoder.decode(right.slice(Math.max(0, i - 60), i + 160)))}`
}

/** Validate the unmodified stream first; normalization must never hide a client-rejected chunk. */
export async function validateChunks(raw: string): Promise<void> {
  const chunks = parseSse(raw)
  const schema = uiMessageChunkSchema()
  for (const [index, chunk] of chunks.entries()) {
    const result = await schema.validate?.(chunk)
    if (!result?.success)
      throw new Error(
        `AI SDK rejected chunk ${index}: ${JSON.stringify(result)}`,
      )
  }
  // The real reducer additionally catches references to missing start/tool parts that schemas alone accept.
  const stream = new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(chunk)
      controller.close()
    },
  })
  const expectedErrors = new Set(
    chunks
      .filter((chunk) => chunk.type === 'error')
      .map((chunk) => chunk.errorText),
  )
  const errors: unknown[] = []
  for await (const _message of readUIMessageStream({
    stream,
    onError: (error) => {
      if (!(error instanceof Error) || !expectedErrors.has(error.message))
        errors.push(error)
    },
  })) {
    /* Drain the client reducer. */
  }
  if (errors.length)
    throw new AggregateError(errors, 'AI SDK reducer rejected the stream')
  // This is also the generated reader used by the migrated scheduler; exercise arbitrary UTF-8 splits.
  const bytes = new TextEncoder().encode(raw)
  const segmented = new ReadableStream<Uint8Array>({
    start(controller) {
      for (let offset = 0; offset < bytes.length; offset += 7)
        controller.enqueue(bytes.slice(offset, offset + 7))
      controller.close()
    },
  })
  for await (const _chunk of readUiMessageStream(
    new Response(segmented, {
      headers: { 'Content-Type': 'text/event-stream' },
    }),
  )) {
    /* Drain the app/scheduler transport. */
  }
}

const volatileTimes = new Set([
  'createdAt',
  'updatedAt',
  'lastUsedAt',
  'lastActiveAt',
  'startedAt',
  'completedAt',
  'timestamp',
  'responseTime',
])
export function normalizeHttp(
  value: unknown,
  ids = new Ids(),
  replacements: Array<[string, string]> = [],
): unknown {
  if (typeof value === 'string') {
    let result = value
    for (const [from, to] of replacements) result = result.replaceAll(from, to)
    return result
  }
  if (Array.isArray(value))
    return value.map((item) => normalizeHttp(item, ids, replacements))
  if (!value || typeof value !== 'object') return value
  return Object.fromEntries(
    Object.entries(value)
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([key, item]) => {
        if (volatileTimes.has(key) && typeof item === 'number')
          return [key, '<time>']
        if (
          ['id', 'runId', 'requestId', 'messageId', 'toolCallId'].includes(
            key,
          ) &&
          typeof item === 'string' &&
          /^[0-9a-f]{8}-[0-9a-f-]{27}$/i.test(item)
        )
          return [key, ids.id(key === 'toolCallId' ? 'tool' : 'uuid', item)]
        return [key, normalizeHttp(item, ids, replacements)]
      }),
  )
}
