/**
 * `setInputFiles`' files, prepared as upstream prepares them before it touches the page, except
 * for what needs a disk, which QuickJS lacks: Rust stats the paths on Neo's disk, in the order
 * the script gave them (crates/neo-sdk/neo-playwright/src/action/input_files.rs).
 *
 *   client   convertInputFiles (client/elementHandle.ts at v1.64.1): paths mixed with payloads,
 *            then payloads totalling 50 MiB, both before any channel call, so unprefixed
 *   channel  the validator's payload checks, `<api>: payloads[0].name: expected string, …`
 *   neo      payloads totalling 10 MiB, which claw-sdk's heap cannot carry (NEO_UPLOAD_LIMIT)
 *   server   prepareFilesForUpload (server/fileUploadUtils.ts): the MIME type from the name
 *            with the vendored `mime` table, and each buffer as `buffer.toString('base64')`
 *
 * The last step is why a payload's bytes cross as text: upstream's server makes that text and
 * its injected script `atob`s it in the page, so a buffer that is not a Buffer (a plain
 * Uint8Array's `toString` joins its numbers) fails there, in upstream as in Neo.
 */
import mime from '../../../neo-playwright/third_party/mime/src/index'
import { Buffer } from './buffer'
import { clientError, unprefixedClientError } from './errors'
import type { FilesInput } from './wire'

/** upstream's fileUploadUtils `fileUploadSizeLimit`. */
const UPLOAD_LIMIT = 50 * 1024 * 1024

/**
 * Neo's own limit on a call's payloads, under upstream's. claw-sdk runs a script in a QuickJS
 * heap of at most 128 MiB (config/defaults.rs), and while its bootstrap encodes the op, the heap
 * holds the payload's bytes, their base64, JSON's quoted copy of that and the JSON: five times
 * the bytes. From a script that holds nothing else, 12.38 MiB is the most that reaches the op
 * whole in a 64 MiB heap (js_runner's, half of claw-sdk's). 10 MiB leaves a script room for
 * everything else, and a larger payload fails here with upstream's advice rather than QuickJS's
 * out of memory.
 */
const NEO_UPLOAD_LIMIT = 10 * 1024 * 1024

/**
 * upstream's client and server conversion of `files`, for the method `api` the script
 * called: `{ paths }` as the script wrote them, or `{ payloads }` ready for the page. `[]` is
 * payloads, as upstream makes it.
 */
export function inputFiles(api: string, files: unknown): FilesInput {
  const items: unknown[] = Array.isArray(files) ? files.slice() : [files]
  if (items.some((item) => typeof item === 'string')) {
    const paths = items.filter(
      (item): item is string => typeof item === 'string',
    )
    if (paths.length !== items.length)
      throw unprefixedClientError(
        api,
        'File paths cannot be mixed with buffers',
      )
    return { paths }
  }
  const total = payloadBytes(api, items)
  if (total >= UPLOAD_LIMIT)
    throw unprefixedClientError(
      api,
      'Cannot set buffer larger than 50Mb, please write it to a file and pass its path instead.',
    )
  const checked = items.map((item, index) => checkedPayload(api, item, index))
  if (total >= NEO_UPLOAD_LIMIT)
    throw unprefixedClientError(
      api,
      'Cannot set buffer larger than 10Mb in Neo, please write it to a file and pass its path instead.',
    )
  return {
    payloads: checked.map(({ name, mimeType, buffer }) => ({
      name,
      mimeType: mimeType || mime.getType(name) || 'application/octet-stream',
      buffer: String(
        Reflect.apply(Reflect.get(buffer, 'toString'), buffer, ['base64']),
      ),
    })),
  }
}

/** Path MIME types come from the same vendored table as in-memory payloads. */
export function dropFiles(api: string, files: unknown) {
  const input = inputFiles(api, files)
  if ('payloads' in input) return input
  return {
    paths: input.paths.map((path) => ({
      path,
      mimeType: mime.getType(path) || 'application/octet-stream',
    })),
  }
}

/**
 * upstream's `filePayloadExceedsSizeLimit`: `item.buffer ? item.buffer.byteLength : 0` summed,
 * with JavaScript's own arithmetic on whatever a buffer's `byteLength` is. Reading `buffer` off
 * a missing payload throws V8's TypeError, before any channel call, as upstream's does.
 */
function payloadBytes(api: string, items: unknown[]): number {
  let total = 0
  for (const item of items) {
    if (item === null || item === undefined)
      throw unprefixedClientError(
        api,
        `Cannot read properties of ${item} (reading 'buffer')`,
        'TypeError',
      )
    const buffer: unknown = Reflect.get(Object(item), 'buffer')
    if (!buffer) continue
    const length: number = Reflect.get(Object(buffer), 'byteLength')
    total += length
  }
  return total
}

/** A payload the channel's validator passed: its string fields and its buffer object. */
type CheckedPayload = { name: string; mimeType?: string; buffer: object }

/**
 * The channel's validator on one payload (`{ name: tString, mimeType: tOptional(tString),
 * buffer: tBinary }`), field by field in that order, with its messages. An in-process client
 * validates binary as `buffer` mode, so any object passes as a buffer.
 */
function checkedPayload(
  api: string,
  item: unknown,
  index: number,
): CheckedPayload {
  const at = `payloads[${index}]`
  if (item === null) throw clientError(api, `${at}: expected object, got null`)
  if (typeof item !== 'object')
    throw clientError(api, `${at}: expected object, got ${typeof item}`)
  const name = validString(api, `${at}.name`, Reflect.get(item, 'name'))
  const rawType: unknown = Reflect.get(item, 'mimeType')
  const mimeType =
    rawType === undefined
      ? undefined
      : validString(api, `${at}.mimeType`, rawType)
  const buffer: unknown = Reflect.get(item, 'buffer')
  if (!(buffer instanceof Buffer) && !(buffer instanceof Object))
    throw clientError(
      api,
      `${at}.buffer: expected Buffer, got ${typeof buffer}`,
    )
  return mimeType === undefined ? { name, buffer } : { name, mimeType, buffer }
}

/** The validator's `tString`: a string, or a String object's own. */
function validString(api: string, path: string, value: unknown): string {
  if (value instanceof String) return value.valueOf()
  if (typeof value === 'string') return value
  throw clientError(api, `${path}: expected string, got ${typeof value}`)
}

/**
 * upstream's client errors that Rust raises for the client, because they need Neo's disk:
 * Node's stat errors, the two of `resolvePathsAndDirectoryForInputFiles`, and Node's refusal of
 * a path with a NUL byte. The prelude's error shaping starts every op error with the method's
 * name, which upstream's client never puts on these, so it is taken off again here. The server's
 * file errors (`ENOENT: … access '…'`) keep it, as upstream's do.
 */
const CLIENT_MESSAGES = [
  /^Multiple directories are not supported$/,
  /^Dropping a directory is not supported/,
  /^File paths must be all files or a single directory$/,
  /^(?:E[A-Z0-9]+|UNKNOWN): [^,\n]+, stat '/,
]
const NUL_PATH =
  /^The argument 'path' must be a string, Uint8Array, or URL without null bytes\. Received /

/**
 * Runs the op `call` for `setInputFiles` from method `api`, giving back the client's errors
 * Rust raised with upstream's shape. The error stays the bootstrap's own object, so the run
 * still knows it as the op's failure.
 */
export async function withClientErrors<T>(
  api: string,
  call: () => Promise<T>,
): Promise<T> {
  try {
    return await call()
  } catch (error) {
    if (!(error instanceof Error) || !error.message.startsWith(`${api}: `))
      throw error
    const bare = error.message.slice(api.length + 2)
    if (NUL_PATH.test(bare)) {
      error.message = bare
      error.name = 'TypeError'
    } else if (CLIENT_MESSAGES.some((message) => message.test(bare)))
      error.message = bare
    throw error
  }
}
