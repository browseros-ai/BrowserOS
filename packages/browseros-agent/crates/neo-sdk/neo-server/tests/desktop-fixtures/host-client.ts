/** The suite's private host transport validates Unix HTTP data before exposing shared model types. */
import { isAbsolute } from 'node:path'
import { jsonObject, numberField, stringField } from './json'
import {
  type HostBytes,
  type HostClient,
  isRunId,
  type JsonObject,
  type RunDownload,
  type RunRecord,
} from './model'

function naturalNumber(value: number, label: string): number {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error(`${label} must be a nonnegative safe integer`)
  }
  return value
}

function downloadRecord(input: unknown): RunDownload {
  const object = jsonObject(input, 'Host download record')
  const download: RunDownload = {
    index: naturalNumber(numberField(object, 'index'), 'Download index'),
    state: stringField(object, 'state'),
    suggestedFilename: stringField(object, 'suggestedFilename'),
  }
  if ('path' in object) download.path = stringField(object, 'path')
  if ('size' in object) {
    download.size = naturalNumber(numberField(object, 'size'), 'Download size')
  }
  return download
}

function runRecord(input: unknown): RunRecord {
  const object = jsonObject(input, 'Host run record')
  const id = stringField(object, 'id')
  if (!isRunId(id)) throw new Error('Host run record omitted a valid run ID')
  if (typeof object.complete !== 'boolean') {
    throw new Error('Host run record complete must be a boolean')
  }
  const truncated = 'resultTruncated' in object ? object.resultTruncated : false
  if (typeof truncated !== 'boolean') {
    throw new Error('Host run record resultTruncated must be a boolean')
  }
  const downloads = 'downloads' in object ? object.downloads : []
  if (!Array.isArray(downloads)) {
    throw new Error('Host run record downloads must be an array')
  }
  const record: RunRecord = {
    id,
    status: stringField(object, 'status'),
    complete: object.complete,
    resultTruncated: truncated,
    downloads: downloads.map(downloadRecord),
  }
  if ('result' in object) record.result = object.result
  return record
}

function settingsState(input: unknown, changing: boolean): JsonObject {
  const object = jsonObject(input, 'Host settings response')
  if (changing) {
    for (const field of ['unknown', 'unknownValues']) {
      const skipped = object[field]
      if (!Array.isArray(skipped) || skipped.length !== 0) {
        throw new Error(`Host settings change skipped inputs in ${field}`)
      }
    }
  }
  return jsonObject(object.settings, 'Host settings')
}

/** Only the private Unix listener receives the host bearer token and mutable settings. */
export function createHostClient({
  socketPath,
  token,
}: {
  socketPath: string
  token: string
}): HostClient {
  if (!isAbsolute(socketPath))
    throw new Error('Host socket path must be absolute')
  if (!/^[\x21-\x7e]{32,}$/.test(token)) {
    throw new Error(
      'Host bearer token must contain at least 32 visible ASCII bytes',
    )
  }

  async function request(method: string, path: string, body?: JsonObject) {
    // The URL supplies HTTP routing only. The mandatory Unix socket and refused redirects
    // prevent the host credential from ever following a response onto a TCP listener.
    const response = await fetch(`http://localhost${path}`, {
      unix: socketPath,
      method,
      redirect: 'error',
      signal: AbortSignal.timeout(45_000),
      headers: {
        authorization: `Bearer ${token}`,
        ...(body ? { 'content-type': 'application/json' } : {}),
      },
      body: body ? JSON.stringify(body) : undefined,
    })
    if (response.status !== 200) {
      await response.body?.cancel()
      throw new Error(`Host ${method} ${path} returned HTTP ${response.status}`)
    }
    return response
  }

  async function bytes(path: string): Promise<HostBytes> {
    const response = await request('GET', path)
    const data = new Uint8Array(await response.arrayBuffer())
    const length = response.headers.get('content-length')
    if (length === null) return { bytes: data }
    if (!/^\d+$/.test(length)) throw new Error('Host Content-Length is invalid')
    return {
      bytes: data,
      contentLength: naturalNumber(Number(length), 'Host Content-Length'),
    }
  }

  return {
    async settings() {
      return settingsState(
        await (await request('GET', '/v1/settings')).json(),
        false,
      )
    },
    async patchSettings(patch) {
      return settingsState(
        await (await request('PATCH', '/v1/settings', patch)).json(),
        true,
      )
    },
    async restoreSettings(settings) {
      return settingsState(
        await (await request('PUT', '/v1/settings', settings)).json(),
        true,
      )
    },
    async record(runId) {
      return runRecord(
        await (
          await request('GET', `/v1/runs/${encodeURIComponent(runId)}?wait=30`)
        ).json(),
      )
    },
    blob(runId, blobId) {
      if (!blobId) throw new Error('Host blob ID must not be empty')
      return bytes(
        `/v1/runs/${encodeURIComponent(runId)}/blobs/${encodeURIComponent(blobId)}`,
      )
    },
    download(runId, index) {
      naturalNumber(index, 'Host download index')
      return bytes(`/v1/runs/${encodeURIComponent(runId)}/downloads/${index}`)
    },
  }
}
