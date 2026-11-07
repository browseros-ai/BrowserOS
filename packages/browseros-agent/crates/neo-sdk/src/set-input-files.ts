/**
 * Playwright's client file conversion for `setInputFiles` (`client/elementHandle.ts`
 * `convertInputFiles`) and `Locator.drop` (`client/frame.ts` `Frame._drop`), in a realm without
 * a file system: paths travel as the script wrote them, and Rust finds each in the run's
 * artifact directory and tells a directory from files (neo-runtime `set_input_files.rs`), where
 * Playwright's client stats them. Buffers travel as the wire's `binary` (`toBinary`).
 */
import type {
  FrameDropParams,
  FrameSetInputFilesParams,
} from '@browseros/neo-runtime-spec/serve'
import { toBinary } from './buffer'

/** Playwright's `FilePayload`. The realm has no Node `Buffer`, so `buffer` is any `Uint8Array`
 * (Neo's `Buffer` from `screenshot()` or `body()` is one). */
export type FilePayload = { name: string; mimeType: string; buffer: Uint8Array }
/** `setInputFiles`' files: paths in the run's artifact directory, or buffers. */
export type InputFiles = string | FilePayload | string[] | FilePayload[]
/** Playwright's `DropPayload`. */
export type DropPayload = {
  files?: InputFiles
  data?: { [mimeType: string]: string }
}

/** Playwright: `client/fileUtils.ts` `fileUploadSizeLimit`: `export const fileUploadSizeLimit = 50 * 1024 * 1024;`
 *
 * Deviation: `client/elementHandle.ts` `convertInputFiles`: `throw new Error('Cannot set buffer larger than 50Mb, please write it to a file and pass its path instead.');` Playwright sends buffers up to this limit; Neo's dispatcher holds at most 8 MiB of a run's requests in flight (neo-runtime `dispatch.rs` `MAX_REQUEST_BYTES_IN_FLIGHT`), so buffers past about 6 MiB (base64 grows them by a third) are refused with its Error naming that bound; a path in the artifact directory takes any size. */
const FILE_UPLOAD_SIZE_LIMIT = 50 * 1024 * 1024

type WirePayloads = NonNullable<FrameSetInputFilesParams['payloads']>

/** Playwright: `client/elementHandle.ts` `convertInputFiles`: `const items: (string | FilePayload)[] = Array.isArray(files) ? files.slice() : [files];` */
export function convertInputFiles(
  files: InputFiles,
): Pick<FrameSetInputFilesParams, 'payloads' | 'localPaths'> {
  const items: unknown[] = Array.isArray(files) ? files.slice() : [files]
  if (items.some((item) => typeof item === 'string')) {
    if (!items.every((item) => typeof item === 'string'))
      throw new Error('File paths cannot be mixed with buffers')
    // Rust resolves each path in the run's artifact directory (`Artifacts::existing`).
    return { localPaths: items as string[] }
  }
  const payloads = items as FilePayload[]
  if (filePayloadExceedsSizeLimit(payloads))
    throw new Error(
      'Cannot set buffer larger than 50Mb, please write it to a file and pass its path instead.',
    )
  return { payloads: payloads.map(wirePayload) }
}

/** Playwright: `client/elementHandle.ts` `filePayloadExceedsSizeLimit`: `return payloads.reduce((size, item) => size + (item.buffer ? item.buffer.byteLength : 0), 0) >= fileUploadSizeLimit;` */
function filePayloadExceedsSizeLimit(payloads: FilePayload[]): boolean {
  return (
    payloads.reduce(
      (size, item) => size + (item.buffer ? item.buffer.byteLength : 0),
      0,
    ) >= FILE_UPLOAD_SIZE_LIMIT
  )
}

/**
 * One payload as the protocol validator leaves it (`tObject` over `name`, `mimeType`,
 * `buffer`, in schema order), its bytes as base64. A `buffer` that is no `Uint8Array` is
 * refused as Playwright's validator refuses one that is no `Buffer`.
 */
function wirePayload(payload: unknown, index: number): WirePayloads[number] {
  const path = `payloads[${index}]`
  if (payload === null) throw new Error(`${path}: expected object, got null`)
  if (typeof payload !== 'object')
    throw new Error(`${path}: expected object, got ${typeof payload}`)
  const { name, mimeType, buffer } = payload as Record<string, unknown>
  if (typeof name !== 'string')
    throw new Error(`${path}.name: expected string, got ${typeof name}`)
  if (mimeType !== undefined && typeof mimeType !== 'string')
    throw new Error(`${path}.mimeType: expected string, got ${typeof mimeType}`)
  if (!(buffer instanceof Uint8Array))
    throw new Error(`${path}.buffer: expected Buffer, got ${typeof buffer}`)
  return mimeType === undefined
    ? { name, buffer: toBinary(buffer) }
    : { name, mimeType, buffer: toBinary(buffer) }
}

/** Playwright: `client/frame.ts` `Frame._drop`: `const dataArray = payload.data ? Object.entries(payload.data).map(([mimeType, value]) => ({ mimeType, value })) : undefined;` */
export function convertDropPayload(
  payload: DropPayload,
): Pick<FrameDropParams, 'payloads' | 'localPaths' | 'data'> {
  let fileParams: Pick<FrameDropParams, 'payloads' | 'localPaths'> = {}
  if (payload.files !== undefined) {
    // A directory among the paths is refused by Rust, which alone can tell (`drop_on.rs`).
    const converted = convertInputFiles(payload.files)
    fileParams = {
      payloads: converted.payloads,
      localPaths: converted.localPaths,
    }
  }
  const data = payload.data
    ? Object.entries(payload.data).map(([mimeType, value], index) => {
        if (typeof value !== 'string')
          throw new Error(
            `data[${index}].value: expected string, got ${typeof value}`,
          )
        return { mimeType, value }
      })
    : undefined
  return { ...fileParams, data }
}
