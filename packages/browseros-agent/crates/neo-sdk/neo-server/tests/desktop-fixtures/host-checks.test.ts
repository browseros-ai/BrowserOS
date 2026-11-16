/** Offline host evidence checks exercise Unix transport, exact bytes, and fail-closed postconditions. */
import { describe, expect, test } from 'bun:test'
import { Buffer } from 'node:buffer'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { checkHost } from './host-checks'
import { createHostClient } from './host-client'
import {
  type HostCheck,
  type HostClient,
  isRunId,
  type JsonValue,
  type RunDownload,
  type RunError,
  type RunId,
  type RunRecord,
  type RunReply,
} from './model'

function testRunId(value = '11111111-2222-3333-4444-555555555555'): RunId {
  if (!isRunId(value)) throw new Error('Invalid test run ID')
  return value
}

type SuccessReply = Extract<RunReply, { kind: 'ok' }>
function success(
  value: JsonValue,
  overrides: Partial<Pick<SuccessReply, 'status' | 'text' | 'images'>> = {},
): SuccessReply {
  return {
    kind: 'ok',
    isError: false,
    runId: testRunId(),
    status: 'ok',
    value,
    text: '',
    images: [],
    raw: {},
    ...overrides,
  }
}

function failure(
  overrides: Partial<RunError> = {},
  status = 'error',
): RunReply {
  return {
    kind: 'error',
    isError: true,
    runId: testRunId(),
    status,
    text: '',
    images: [],
    raw: {},
    error: {
      owner: 'script',
      code: 'script_error',
      message: 'Error: run-boom (run:5:103)',
      ...overrides,
    },
  }
}

function recordFor(
  reply: RunReply,
  overrides: Partial<RunRecord> = {},
): RunRecord {
  return {
    id: reply.runId,
    status: reply.status,
    complete: true,
    resultTruncated: false,
    downloads: [],
    ...overrides,
  }
}

function hostWith(
  record: RunRecord,
  overrides: Partial<HostClient> = {},
): HostClient {
  return {
    async settings() {
      throw new Error('Unexpected settings read')
    },
    async patchSettings() {
      throw new Error('Unexpected settings patch')
    },
    async restoreSettings() {
      throw new Error('Unexpected settings restore')
    },
    async record() {
      return record
    },
    async blob() {
      throw new Error('Unexpected blob read')
    },
    async download() {
      throw new Error('Unexpected download read')
    },
    ...overrides,
  }
}

function inline(bytes: Uint8Array): JsonValue {
  return { $neo: 'bytes', base64: Buffer.from(bytes).toString('base64') }
}

const png = Buffer.from('89504e470d0a1a0a010203040506', 'hex')
const pdf = Buffer.from('%PDF-1.7\nfixture\n')
const captureSpec: HostCheck = {
  kind: 'capture-bytes',
  magicHex: '89504e47',
  mimeType: 'image/png',
}
const logSpec: HostCheck = {
  kind: 'run-log',
  text: 'run-log-line',
  value: 'done',
}
const errorSpec: HostCheck = {
  kind: 'run-error',
  messageIncludes: 'run-boom',
  requireIsError: true,
}
const nestedSpec: HostCheck = {
  kind: 'nested-capture-exact-bytes',
  fields: ['screenshot', 'pdf'],
  sizes: { screenshot: 'screenshotBytes', pdf: 'pdfBytes' },
  requiredValue: { done: true },
  compareReturnedAndPersistedEveryByte: true,
  sameRunId: true,
}

describe('authenticated Unix host boundary', () => {
  test('authenticates every route and parses settings, records, and byte reads', async () => {
    const root = await mkdtemp(join(tmpdir(), 'neo-host-'))
    const token = 'desktop-fixture-test-bearer-token-123456'
    const seen: { method: string; path: string; body: string }[] = []
    const server = Bun.serve({
      unix: join(root, 'host.sock'),
      async fetch(request) {
        expect(request.headers.get('authorization')).toBe(`Bearer ${token}`)
        const url = new URL(request.url)
        seen.push({
          method: request.method,
          path: url.pathname + url.search,
          body: await request.text(),
        })
        if (url.pathname === '/v1/settings') {
          return Response.json({
            settings: { trace: { valueBytes: 262144 } },
            ...(request.method === 'GET'
              ? {}
              : { unknown: [], unknownValues: [] }),
          })
        }
        if (
          url.pathname.endsWith('/blobs/a%2Fb') ||
          url.pathname.endsWith('/downloads/0')
        )
          return new Response(png)
        return Response.json({
          id: testRunId(),
          status: 'ok',
          complete: true,
          result: null,
        })
      },
    })
    try {
      const host = createHostClient({
        socketPath: join(root, 'host.sock'),
        token,
      })
      expect(await host.settings()).toEqual({ trace: { valueBytes: 262144 } })
      await host.patchSettings({ trace: { valueBytes: 262144 } })
      await host.restoreSettings({ trace: { valueBytes: 262144 } })
      expect(await host.record(testRunId())).toEqual({
        id: testRunId(),
        status: 'ok',
        complete: true,
        result: null,
        resultTruncated: false,
        downloads: [],
      })
      expect((await host.blob(testRunId(), 'a/b')).bytes).toEqual(
        new Uint8Array(png),
      )
      expect((await host.download(testRunId(), 0)).bytes).toEqual(
        new Uint8Array(png),
      )
      expect(seen.map((item) => item.method)).toEqual([
        'GET',
        'PATCH',
        'PUT',
        'GET',
        'GET',
        'GET',
      ])
      expect(seen[1]?.body).toBe('{"trace":{"valueBytes":262144}}')
      expect(seen[3]?.path).toBe(`/v1/runs/${testRunId()}?wait=30`)
      expect(seen[4]?.path).toBe(`/v1/runs/${testRunId()}/blobs/a%2Fb`)
    } finally {
      await server.stop(true)
      await rm(root, { recursive: true, force: true })
    }
  })

  test('rejects malformed host records, skipped settings, HTTP errors, and redirects', async () => {
    const root = await mkdtemp(join(tmpdir(), 'neo-host-'))
    let response = () => Response.json({})
    const server = Bun.serve({
      unix: join(root, 'host.sock'),
      fetch() {
        return response()
      },
    })
    try {
      const host = createHostClient({
        socketPath: join(root, 'host.sock'),
        token: 'desktop-fixture-test-bearer-token-123456',
      })
      const valid = { id: testRunId(), status: 'ok', complete: true }
      const invalid: JsonValue[] = [
        { ...valid, id: 'caller-task' },
        { ...valid, complete: 'true' },
        { ...valid, resultTruncated: null },
        { ...valid, downloads: null },
        {
          ...valid,
          downloads: [
            { index: -1, state: 'completed', suggestedFilename: 'report' },
          ],
        },
        {
          ...valid,
          downloads: [
            {
              index: 0,
              state: 'completed',
              suggestedFilename: 'report',
              size: -1,
            },
          ],
        },
      ]
      for (const value of invalid) {
        response = () => Response.json(value)
        await expect(host.record(testRunId())).rejects.toThrow()
      }
      response = () =>
        Response.json({
          settings: {},
          unknown: ['trace.typo'],
          unknownValues: [],
        })
      await expect(
        host.patchSettings({ trace: { valueBytes: 262144 } }),
      ).rejects.toThrow('skipped inputs')
      response = () => new Response('denied', { status: 403 })
      await expect(host.settings()).rejects.toThrow('HTTP 403')
      response = () => Response.redirect('http://127.0.0.1:9/credential-trap')
      await expect(host.settings()).rejects.toThrow()
    } finally {
      await server.stop(true)
      await rm(root, { recursive: true, force: true })
    }
    expect(() =>
      createHostClient({ socketPath: 'relative.sock', token: 'x'.repeat(32) }),
    ).toThrow('absolute')
    expect(() =>
      createHostClient({ socketPath: '/tmp/unused.sock', token: 'short' }),
    ).toThrow('32 visible')
  })
})

describe('capture and persistence host evidence', () => {
  test('accepts real format signatures and checks exact persisted PDF bytes', async () => {
    const variants: { spec: HostCheck; bytes: Uint8Array }[] = [
      { spec: captureSpec, bytes: png },
      {
        spec: {
          kind: 'capture-bytes',
          magicHex: 'ffd8',
          mimeType: 'image/jpeg',
        },
        bytes: Buffer.from('ffd801020304', 'hex'),
      },
      {
        spec: {
          kind: 'capture-bytes',
          magicHex: '52494646',
          mimeType: 'image/webp',
        },
        bytes: Buffer.from('RIFFabcdWEBPpayload'),
      },
      {
        spec: {
          kind: 'capture-bytes',
          magicUtf8: '%PDF-',
          requirePersistedExactBytes: true,
        },
        bytes: pdf,
      },
    ]
    for (const variant of variants) {
      const value = {
        bytes: inline(variant.bytes),
        length: variant.bytes.length,
      }
      const reply = success(value)
      await checkHost({
        spec: variant.spec,
        replies: [reply],
        host: hostWith(recordFor(reply, { result: value })),
      })
    }
  })

  test('rejects shortened PNG signatures, RIFF of another type, and invalid JPEG/PDF magic', async () => {
    const variants: { spec: HostCheck; bytes: Uint8Array }[] = [
      { spec: captureSpec, bytes: Buffer.from('89504e470000000000', 'hex') },
      {
        spec: {
          kind: 'capture-bytes',
          magicHex: '52494646',
          mimeType: 'image/webp',
        },
        bytes: Buffer.from('RIFFabcdWAVEpayload'),
      },
      {
        spec: {
          kind: 'capture-bytes',
          magicHex: 'ffd8',
          mimeType: 'image/jpeg',
        },
        bytes: png,
      },
      {
        spec: { kind: 'capture-bytes', magicUtf8: '%PDF-' },
        bytes: Buffer.from('%PDX-invalid'),
      },
    ]
    for (const variant of variants) {
      const reply = success({
        bytes: inline(variant.bytes),
        length: variant.bytes.length,
      })
      await expect(
        checkHost({
          spec: variant.spec,
          replies: [reply],
          host: hostWith(recordFor(reply)),
        }),
      ).rejects.toThrow()
    }
  })

  test('rejects noncanonical base64 and reported length mismatches', async () => {
    for (const base64 of ['???', 'AA=', 'AB==', 'SGVsbG8', 'AA==\n']) {
      const reply = success({ bytes: { $neo: 'bytes', base64 } })
      await expect(
        checkHost({
          spec: captureSpec,
          replies: [reply],
          host: hostWith(recordFor(reply)),
        }),
      ).rejects.toThrow('base64')
    }
    const reply = success({ bytes: inline(png), length: png.length + 1 })
    await expect(
      checkHost({
        spec: captureSpec,
        replies: [reply],
        host: hostWith(recordFor(reply)),
      }),
    ).rejects.toThrow('Reported capture length')
  })

  test('resolves blob bytes only under the producing run and checks marker/HTTP lengths', async () => {
    const reply = success({
      bytes: { $neo: 'blob', id: 'blob-1', size: png.length },
      length: png.length,
    })
    const calls: { runId: RunId; blobId: string }[] = []
    const host = hostWith(recordFor(reply), {
      async blob(runId, blobId) {
        calls.push({ runId, blobId })
        return { bytes: png, contentLength: png.length }
      },
    })
    await checkHost({ spec: captureSpec, replies: [reply], host })
    expect(calls).toEqual([{ runId: reply.runId, blobId: 'blob-1' }])
    for (const bytes of [
      { bytes: png, contentLength: png.length + 1 },
      { bytes: png.subarray(0, -1) },
    ]) {
      await expect(
        checkHost({
          spec: captureSpec,
          replies: [reply],
          host: hostWith(recordFor(reply), {
            async blob() {
              return bytes
            },
          }),
        }),
      ).rejects.toThrow()
    }
  })

  test('every host check rejects wrong run IDs, incomplete records, and wrong statuses', async () => {
    const reply = success({ bytes: inline(png), length: png.length })
    for (const changes of [
      { id: testRunId('aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee') },
      { complete: false },
      { status: 'error' },
    ]) {
      await expect(
        checkHost({
          spec: captureSpec,
          replies: [reply],
          host: hostWith(recordFor(reply, changes)),
        }),
      ).rejects.toThrow()
    }
    const wrongStatus = success(reply.value, { status: 'running' })
    await expect(
      checkHost({
        spec: captureSpec,
        replies: [wrongStatus],
        host: hostWith(recordFor(wrongStatus)),
      }),
    ).rejects.toThrow('metadata')
    await expect(
      checkHost({
        spec: captureSpec,
        replies: [],
        host: hostWith(recordFor(reply)),
      }),
    ).rejects.toThrow('no raw MCP')
  })

  test('persistence rejects missing/truncated results and differences beyond the signature', async () => {
    const spec: HostCheck = {
      kind: 'capture-bytes',
      magicUtf8: '%PDF-',
      requirePersistedExactBytes: true,
    }
    const reply = success({ bytes: inline(pdf), length: pdf.length })
    const changed = Buffer.from(pdf)
    changed[changed.length - 1] = 0
    for (const changes of [
      {},
      { result: reply.value, resultTruncated: true },
      { result: { bytes: inline(changed), length: changed.length } },
    ]) {
      await expect(
        checkHost({
          spec,
          replies: [reply],
          host: hostWith(recordFor(reply, changes)),
        }),
      ).rejects.toThrow()
    }
  })

  test('nested captures compare both byte values and sizes from the same persisted run', async () => {
    const value = {
      done: true,
      screenshot: inline(png),
      pdf: inline(pdf),
      screenshotBytes: png.length,
      pdfBytes: pdf.length,
    }
    const reply = success(value)
    await checkHost({
      spec: nestedSpec,
      replies: [reply],
      host: hostWith(recordFor(reply, { result: value })),
    })
    for (const stored of [
      { ...value, done: false },
      { ...value, pdfBytes: pdf.length + 1 },
      {
        ...value,
        screenshot: inline(
          Buffer.concat([png.subarray(0, -1), Buffer.from([0])]),
        ),
      },
    ]) {
      await expect(
        checkHost({
          spec: nestedSpec,
          replies: [reply],
          host: hostWith(recordFor(reply, { result: stored })),
        }),
      ).rejects.toThrow()
    }
  })

  test('typed JSON retains the complete Unicode payload and rejects extra or clipped fields', async () => {
    const spec: HostCheck = {
      kind: 'typed-json',
      count: 7,
      textRepeat: 'line\n🦀 ',
      times: 2000,
      requirePersistedResult: true,
    }
    const value = { count: 7, text: 'line\n🦀 '.repeat(2000) }
    const reply = success(value)
    await checkHost({
      spec,
      replies: [reply],
      host: hostWith(recordFor(reply, { result: value })),
    })
    for (const result of [
      { ...value, extra: true },
      { ...value, text: value.text.slice(0, -1) },
    ]) {
      await expect(
        checkHost({
          spec,
          replies: [reply],
          host: hostWith(recordFor(reply, { result })),
        }),
      ).rejects.toThrow('Persisted typed JSON')
    }
    await expect(
      checkHost({
        spec,
        replies: [success({ ...value, extra: true })],
        host: hostWith(recordFor(reply, { result: value })),
      }),
    ).rejects.toThrow('Returned typed JSON')
  })
})

describe('console and intentional errors', () => {
  test('accepts standalone console output and refuses return JSON impersonating logs', async () => {
    const reply = success('done', {
      text: '"done"\nconsole:\nrun-log-line\nTip: following note',
    })
    await checkHost({
      spec: logSpec,
      replies: [reply],
      host: hostWith(recordFor(reply)),
    })
    for (const text of [
      JSON.stringify({ data: 'console:\nrun-log-line' }),
      '"run-log-line"',
      '"done"\nconsole:\n[2 console lines were dropped]\nrun-log-line',
    ]) {
      await expect(
        checkHost({
          spec: logSpec,
          replies: [success('done', { text })],
          host: hostWith(recordFor(reply)),
        }),
      ).rejects.toThrow()
    }
    await expect(
      checkHost({
        spec: logSpec,
        replies: [success('wrong', { text: reply.text })],
        host: hostWith(recordFor(reply)),
      }),
    ).rejects.toThrow('wrong value')
  })

  test('run-boom is a passing error protocol assertion only for script/script_error', async () => {
    const reply = failure()
    await checkHost({
      spec: errorSpec,
      replies: [reply],
      host: hostWith(recordFor(reply)),
    })
    for (const wrong of [
      failure({ owner: 'host' }),
      failure({ code: 'wait_timeout' }),
      failure({ message: 'Error: different problem' }),
      failure({}, 'ok'),
      success(null),
    ]) {
      await expect(
        checkHost({
          spec: errorSpec,
          replies: [wrong],
          host: hostWith(recordFor(wrong)),
        }),
      ).rejects.toThrow()
    }
  })
})

describe('downloads retain disk and same-run service evidence', () => {
  test('exact ruled fixture bytes reject extra bytes even when every same-run copy agrees', async () => {
    const root = await mkdtemp(join(tmpdir(), 'neo-download-exact-'))
    const path = join(root, 'report.txt')
    const expected = 'fixture report contents\n'
    const bytes = Buffer.from(`${expected}extra`)
    try {
      await writeFile(path, bytes)
      const reply = success({ path })
      const host = hostWith(
        recordFor(reply, {
          downloads: [
            {
              index: 0,
              path,
              size: bytes.length,
              state: 'completed',
              suggestedFilename: 'report.txt',
            },
          ],
        }),
        {
          async download() {
            return { bytes }
          },
        },
      )
      await expect(
        checkHost({
          spec: {
            kind: 'download-file',
            pathField: 'path',
            contentIncludes: 'fixture report',
            contentEquals: expected,
            requireExists: true,
          },
          replies: [reply],
          host,
        }),
      ).rejects.toThrow('exact fixture bytes')
    } finally {
      await rm(root, { recursive: true, force: true })
    }
  })
  test('reads each distinct completed download and compares every byte', async () => {
    const root = await mkdtemp(join(tmpdir(), 'neo-download-'))
    const bytes = Buffer.from('fixture report\nexact downloaded bytes\n')
    const paths = [join(root, 'report.txt'), join(root, 'report (1).txt')]
    try {
      await Promise.all(paths.map((path) => writeFile(path, bytes)))
      const downloads = paths.map((path, index) => ({
        path,
        index,
        size: bytes.length,
        state: 'completed',
        suggestedFilename: 'report.txt',
      }))
      const calls: { runId: RunId; index: number }[] = []
      const reply = success({ paths })
      const host = hostWith(recordFor(reply, { downloads }), {
        async download(runId, index) {
          calls.push({ runId, index })
          return { bytes, contentLength: bytes.length }
        },
      })
      await checkHost({
        spec: {
          kind: 'download-files',
          pathsField: 'paths',
          contentIncludes: 'fixture report',
          requireExists: true,
        },
        replies: [reply],
        host,
      })
      expect(calls).toEqual(
        paths.map((_, index) => ({ runId: reply.runId, index })),
      )
      await checkHost({
        spec: {
          kind: 'download-file',
          pathField: 'path',
          contentIncludes: 'fixture report',
          requireExists: true,
        },
        replies: [success({ path: paths[0] ?? '' })],
        host,
      })
    } finally {
      await rm(root, { recursive: true, force: true })
    }
  })

  test('rejects incomplete/duplicate records, invalid sizes/indices, and disk/service differences', async () => {
    const root = await mkdtemp(join(tmpdir(), 'neo-download-'))
    const path = join(root, 'report.txt')
    const bytes = Buffer.from('fixture report\nexact downloaded bytes\n')
    const spec: HostCheck = {
      kind: 'download-file',
      pathField: 'path',
      contentIncludes: 'fixture report',
      requireExists: true,
    }
    const reply = success({ path })
    const download: RunDownload = {
      path,
      index: 0,
      size: bytes.length,
      state: 'completed',
      suggestedFilename: 'report.txt',
    }
    try {
      await writeFile(path, bytes)
      for (const downloads of [
        [],
        [download, download],
        [{ ...download, state: 'inProgress' }],
        [{ ...download, index: -1 }],
        [{ ...download, size: bytes.length + 1 }],
        [{ ...download, size: undefined }],
      ]) {
        await expect(
          checkHost({
            spec,
            replies: [reply],
            host: hostWith(recordFor(reply, { downloads }), {
              async download() {
                return { bytes }
              },
            }),
          }),
        ).rejects.toThrow()
      }
      const changed = Buffer.from(bytes)
      changed[changed.length - 1] = 0
      for (const served of [
        { bytes: changed },
        { bytes, contentLength: bytes.length + 1 },
      ]) {
        await expect(
          checkHost({
            spec,
            replies: [reply],
            host: hostWith(recordFor(reply, { downloads: [download] }), {
              async download() {
                return served
              },
            }),
          }),
        ).rejects.toThrow()
      }
      const wrongContents = Buffer.from('an unrelated download payload')
      await writeFile(path, wrongContents)
      await expect(
        checkHost({
          spec,
          replies: [reply],
          host: hostWith(
            recordFor(reply, {
              downloads: [{ ...download, size: wrongContents.length }],
            }),
            {
              async download() {
                return { bytes: wrongContents }
              },
            },
          ),
        }),
      ).rejects.toThrow('expected fixture contents')
      const repeated = success({ paths: [path, path] })
      await expect(
        checkHost({
          spec: {
            kind: 'download-files',
            pathsField: 'paths',
            contentIncludes: 'fixture report',
            requireExists: true,
          },
          replies: [repeated],
          host: hostWith(recordFor(reply, { downloads: [download] })),
        }),
      ).rejects.toThrow('same file path')
      await rm(path)
      await expect(
        checkHost({
          spec,
          replies: [reply],
          host: hostWith(recordFor(reply, { downloads: [download] })),
        }),
      ).rejects.toThrow()
    } finally {
      await rm(root, { recursive: true, force: true })
    }
  })
})
