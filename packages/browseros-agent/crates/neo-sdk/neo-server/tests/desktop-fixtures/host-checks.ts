/** Host assertions bind returned values, retained records, and byte artifacts to one producing run. */
import { Buffer } from 'node:buffer'
import { readFile, stat } from 'node:fs/promises'
import { canonicalJson, jsonObject, numberField, stringField } from './json'
import type {
  HostBytes,
  HostCheck,
  HostClient,
  JsonObject,
  JsonValue,
  RunId,
  RunRecord,
  RunReply,
} from './model'

type SuccessReply = Extract<RunReply, { kind: 'ok' }>
type CheckKind<K extends HostCheck['kind']> = Extract<HostCheck, { kind: K }>

function requireCheck(condition: boolean, message: string): asserts condition {
  if (!condition) throw new Error(message)
}

function fieldValue(object: JsonObject, field: string): JsonValue {
  requireCheck(
    field in object,
    `Returned value omitted required field ${field}`,
  )
  return object[field]
}

function sameJson(actual: JsonValue, expected: JsonValue, message: string) {
  requireCheck(canonicalJson(actual) === canonicalJson(expected), message)
}

function sameBytes(actual: Uint8Array, expected: Uint8Array, message: string) {
  requireCheck(Buffer.from(actual).equals(expected), message)
}

function strictBase64(encoded: string): Uint8Array {
  const valid =
    /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/
  requireCheck(valid.test(encoded), 'Inline byte marker has invalid base64')
  const bytes = Buffer.from(encoded, 'base64')
  requireCheck(
    bytes.toString('base64') === encoded,
    'Inline byte marker has noncanonical base64',
  )
  return bytes
}

function checkHttpLength(result: HostBytes, label: string) {
  if (result.contentLength !== undefined) {
    requireCheck(
      Number.isSafeInteger(result.contentLength) &&
        result.contentLength === result.bytes.length,
      `${label} HTTP length differs from its bytes`,
    )
  }
}

async function resolveBytes({
  marker,
  runId,
  host,
  field,
}: {
  marker: JsonValue
  runId: RunId
  host: HostClient
  field: string
}): Promise<Uint8Array> {
  const object = jsonObject(marker, `Capture ${field}`)
  if (object.$neo === 'bytes')
    return strictBase64(stringField(object, 'base64'))
  requireCheck(
    object.$neo === 'blob',
    `Capture ${field} is not an inline byte or blob marker`,
  )
  const id = stringField(object, 'id')
  const size = numberField(object, 'size')
  requireCheck(
    id.length > 0 && Number.isSafeInteger(size) && size >= 0,
    'Blob marker has invalid ID or size',
  )
  // Blob IDs are scoped by the producing run. A fresh capture or another run's blob
  // could have the same header while silently weakening persistence assertions.
  const result = await host.blob(runId, id)
  checkHttpLength(result, 'Blob')
  requireCheck(
    result.bytes.length === size,
    'Host blob byte count differs from its marker',
  )
  return result.bytes
}

async function completeRecord(
  reply: RunReply,
  host: HostClient,
): Promise<RunRecord> {
  const record = await host.record(reply.runId)
  requireCheck(record.id === reply.runId, 'Host lookup returned another run')
  requireCheck(record.complete, 'Host run has not finished persisting')
  const expectedStatus = reply.kind === 'ok' ? 'ok' : 'error'
  requireCheck(
    reply.status === expectedStatus,
    `Reply metadata must say ${expectedStatus}`,
  )
  requireCheck(
    record.status === expectedStatus,
    `Host run status must say ${expectedStatus}`,
  )
  return record
}

function successful(reply: RunReply): asserts reply is SuccessReply {
  requireCheck(
    reply.kind === 'ok' && !reply.isError,
    'A success host check received an error reply',
  )
}

function persistedValue(record: RunRecord): JsonValue {
  requireCheck(!record.resultTruncated, 'Persisted result is truncated')
  const result = record.result
  requireCheck(result !== undefined, 'Host record omitted persisted result')
  return result
}

function startsWith(bytes: Uint8Array, prefix: Uint8Array): boolean {
  return (
    bytes.length > prefix.length &&
    prefix.every((value, index) => bytes[index] === value)
  )
}

function captureSignature(bytes: Uint8Array, mime: string) {
  switch (mime) {
    case 'image/png':
      requireCheck(
        startsWith(bytes, Buffer.from('89504e470d0a1a0a', 'hex')),
        'Capture is missing the full PNG signature',
      )
      return
    case 'image/jpeg':
      requireCheck(
        startsWith(bytes, Buffer.from('ffd8', 'hex')),
        'Capture is missing the JPEG signature',
      )
      return
    case 'image/webp':
      requireCheck(
        startsWith(bytes, Buffer.from('RIFF')) &&
          Buffer.from(bytes.subarray(8, 12)).equals(Buffer.from('WEBP')),
        'Capture is missing the RIFF/WEBP signature',
      )
      return
    case 'application/pdf':
      requireCheck(
        startsWith(bytes, Buffer.from('%PDF-')),
        'Capture is missing the PDF signature',
      )
      return
    default:
      throw new Error(`Unsupported capture MIME type ${mime}`)
  }
}

async function captureBytes(
  spec: CheckKind<'capture-bytes'>,
  reply: SuccessReply,
  record: RunRecord,
  host: HostClient,
) {
  const value = jsonObject(reply.value, 'Capture result')
  const data = await resolveBytes({
    marker: fieldValue(value, 'bytes'),
    runId: reply.runId,
    host,
    field: 'bytes',
  })
  if (spec.magicHex !== undefined) {
    requireCheck(
      /^(?:[a-fA-F0-9]{2})+$/.test(spec.magicHex),
      'Capture magicHex must contain complete hex bytes',
    )
  }
  const magic =
    spec.magicHex !== undefined
      ? Buffer.from(spec.magicHex, 'hex')
      : Buffer.from(spec.magicUtf8 ?? '')
  requireCheck(
    magic.length > 0 && startsWith(data, magic),
    'Returned capture is missing required magic bytes',
  )
  if ('length' in value)
    requireCheck(
      numberField(value, 'length') === data.length,
      'Reported capture length differs from actual bytes',
    )
  if (spec.mimeType !== undefined) captureSignature(data, spec.mimeType)
  if (spec.magicUtf8 === '%PDF-') captureSignature(data, 'application/pdf')
  if (spec.requireImageBlock) {
    const images = reply.images.filter(
      (image) => image.mimeType === spec.mimeType,
    )
    requireCheck(
      images.length > 0,
      'MCP reply omitted the required native image block',
    )
    requireCheck(
      images.some((image) =>
        Buffer.from(strictBase64(image.data)).equals(data),
      ),
      'Native image block differs from returned bytes',
    )
  }
  if (spec.requirePersistedExactBytes) {
    const stored = jsonObject(
      persistedValue(record),
      'Persisted capture result',
    )
    const saved = await resolveBytes({
      marker: fieldValue(stored, 'bytes'),
      runId: reply.runId,
      host,
      field: 'bytes',
    })
    sameBytes(
      saved,
      data,
      'Returned and persisted capture differ byte for byte',
    )
    if ('length' in stored)
      requireCheck(
        numberField(stored, 'length') === saved.length,
        'Persisted capture length differs from actual bytes',
      )
  }
}

async function nestedCapture(
  spec: CheckKind<'nested-capture-exact-bytes'>,
  reply: SuccessReply,
  record: RunRecord,
  host: HostClient,
) {
  const value = jsonObject(reply.value, 'Nested capture result')
  const stored = jsonObject(
    persistedValue(record),
    'Persisted nested capture result',
  )
  for (const [key, expected] of Object.entries(spec.requiredValue)) {
    sameJson(
      fieldValue(value, key),
      expected,
      `Nested capture lost required scalar ${key}`,
    )
    sameJson(
      fieldValue(stored, key),
      expected,
      `Persisted capture lost required scalar ${key}`,
    )
  }
  for (const field of spec.fields) {
    const sizeField = spec.sizes[field]
    requireCheck(
      typeof sizeField === 'string',
      `Nested capture omitted size field for ${field}`,
    )
    const returned = await resolveBytes({
      marker: fieldValue(value, field),
      runId: reply.runId,
      host,
      field,
    })
    const saved = await resolveBytes({
      marker: fieldValue(stored, field),
      runId: reply.runId,
      host,
      field,
    })
    requireCheck(
      numberField(value, sizeField) === returned.length,
      `Nested capture reported wrong byte count for ${field}`,
    )
    requireCheck(
      numberField(stored, sizeField) === saved.length,
      `Persisted capture reported wrong byte count for ${field}`,
    )
    if (field === 'screenshot') captureSignature(returned, 'image/png')
    if (field === 'pdf') captureSignature(returned, 'application/pdf')
    sameBytes(
      saved,
      returned,
      `Nested returned and persisted ${field} differ byte for byte`,
    )
  }
}

async function downloadFiles(
  spec: CheckKind<'download-file' | 'download-files'>,
  reply: SuccessReply,
  record: RunRecord,
  host: HostClient,
) {
  const value = jsonObject(reply.value, 'Download result')
  const paths =
    spec.kind === 'download-file'
      ? [fieldValue(value, spec.pathField)]
      : fieldValue(value, spec.pathsField)
  requireCheck(
    Array.isArray(paths) && paths.length > 0,
    'Download result omitted paths',
  )
  requireCheck(
    new Set(paths).size === paths.length,
    'Repeated downloads reused the same file path',
  )
  const indices = new Set<number>()
  for (const path of paths) {
    requireCheck(
      typeof path === 'string' && path.length > 0,
      'Download result omitted a valid path',
    )
    const matches = record.downloads.filter(
      (download) => download.path === path,
    )
    requireCheck(
      matches.length === 1,
      'The same run must record each download path exactly once',
    )
    const download = matches[0]
    requireCheck(download !== undefined, 'The run omitted its download')
    requireCheck(
      download.state === 'completed',
      'Recorded download is not completed',
    )
    requireCheck(
      Number.isSafeInteger(download.index) && download.index >= 0,
      'Recorded download omitted a valid index',
    )
    requireCheck(
      !indices.has(download.index),
      'Recorded downloads reused an endpoint index',
    )
    indices.add(download.index)
    requireCheck(
      (await stat(path)).isFile(),
      'Reported download path is not a regular file',
    )
    const disk = await readFile(path)
    const served = await host.download(reply.runId, download.index)
    checkHttpLength(served, 'Download')
    requireCheck(
      download.size === disk.length && disk.length === served.bytes.length,
      'Recorded download size differs from disk or service bytes',
    )
    sameBytes(
      disk,
      served.bytes,
      'Disk and host-service download copies differ byte for byte',
    )
    requireCheck(
      disk.includes(Buffer.from(spec.contentIncludes)),
      'Download omitted expected fixture contents',
    )
    if (spec.kind === 'download-file' && spec.contentEquals !== undefined) {
      sameBytes(
        disk,
        Buffer.from(spec.contentEquals),
        'Download differs from exact fixture bytes',
      )
    }
  }
}

/** Read only the standalone console section, so serialized return data cannot impersonate a log. */
function consoleLines(text: string): string[] {
  const lines = text.split('\n')
  const marker = lines.indexOf('console:')
  if (marker < 0) return []
  const kept: string[] = []
  for (const line of lines.slice(marker + 1)) {
    if (
      line.startsWith('[unknown outcome]') ||
      line.startsWith('[not awaited]') ||
      line.startsWith('Tip:')
    )
      break
    requireCheck(
      !(line.startsWith('[') && line.includes('console lines were dropped')),
      'Run dropped console output',
    )
    kept.push(line)
  }
  return kept
}

/** Host assertions follow the original reply's run ID through persisted JSON, blobs, and disk. */
export async function checkHost({
  spec,
  replies,
  host,
}: {
  spec: HostCheck
  replies: readonly RunReply[]
  host: HostClient
}): Promise<void> {
  requireCheck(replies.length > 0, 'Host check received no raw MCP replies')
  if (spec.kind === 'run-log') {
    for (const reply of replies) {
      successful(reply)
      await completeRecord(reply, host)
      if (consoleLines(reply.text).includes(spec.text)) {
        sameJson(
          reply.value,
          spec.value,
          'The logging run returned the wrong value',
        )
        return
      }
    }
    throw new Error(
      'The exact console line is absent from the raw MCP console section',
    )
  }
  const reply = replies.at(-1)
  requireCheck(reply !== undefined, 'Host check received no reply')
  const record = await completeRecord(reply, host)
  if (spec.kind === 'run-error') {
    requireCheck(
      reply.kind === 'error' && reply.isError,
      'The intentionally throwing run did not set isError',
    )
    requireCheck(
      reply.error.owner === 'script' && reply.error.code === 'script_error',
      'Intentional exception must be a script/script_error',
    )
    requireCheck(
      reply.error.message.includes(spec.messageIncludes),
      'Intentional exception lost its expected message',
    )
    return
  }
  successful(reply)
  switch (spec.kind) {
    case 'capture-bytes':
      await captureBytes(spec, reply, record, host)
      return
    case 'nested-capture-exact-bytes':
      await nestedCapture(spec, reply, record, host)
      return
    case 'download-file':
    case 'download-files':
      await downloadFiles(spec, reply, record, host)
      return
    case 'typed-json': {
      const expected = {
        count: spec.count,
        text: spec.textRepeat.repeat(spec.times),
      }
      sameJson(
        reply.value,
        expected,
        'Returned typed JSON differs from the exact expected object',
      )
      sameJson(
        persistedValue(record),
        expected,
        'Persisted typed JSON differs from the exact expected object',
      )
      return
    }
    default: {
      const exhaustive: never = spec
      throw new Error(`Unimplemented host check ${exhaustive}`)
    }
  }
}
