/** Sends exact program bytes through the public MCP run path and verifies the stored run trace.
 * Failed calls are recorded once; a possibly dispatched browser action is never replayed. */

import { Buffer } from 'node:buffer'
import { readFileSync, writeFileSync } from 'node:fs'
import http from 'node:http'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import { sha } from './upstream.mjs'
export function hostGet(env, path, { signal, timeout = 60000 } = {}) {
  const deadline = AbortSignal.timeout(timeout)
  const bounded = signal ? AbortSignal.any([deadline, signal]) : deadline
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        socketPath: env.hostSocket,
        path,
        headers: { authorization: `Bearer ${env.hostToken}` },
        signal: bounded,
      },
      (res) => {
        let data = ''
        const incomplete = () =>
          reject(
            new Error(
              deadline.aborted
                ? 'host trace deadline exceeded'
                : 'incomplete host trace response',
            ),
          )
        // A partial HTTP body can emit close without end. The deadline spans headers and body
        // even when the peer keeps sending bytes.
        res.once('aborted', incomplete)
        res.once('error', incomplete)
        res.once('close', () => {
          if (!res.complete) incomplete()
        })
        res.setEncoding('utf8')
        res.on('data', (chunk) => {
          data += chunk
        })
        res.once('end', () => {
          try {
            if (!res.complete) throw new Error('incomplete host trace response')
            if (res.statusCode !== 200)
              throw new Error(`host HTTP ${res.statusCode}`)
            resolve(JSON.parse(data))
          } catch (error) {
            reject(error)
          }
        })
      },
    )
    req.once('error', (error) =>
      reject(
        deadline.aborted ? new Error('host trace deadline exceeded') : error,
      ),
    )
    req.end()
  })
}
export async function connectNeo(env, signal) {
  const client = new Client({ name: 'neo-conformance', version: '1' })
  try {
    await client.connect(
      new StreamableHTTPClientTransport(new URL(env.neoMcp)),
      { signal },
    )
    return client
  } catch (error) {
    await client.close()
    throw error
  }
}
// The host trace exposes a bounded source preview, while the MCP call carries the whole
// program. Reproduce neo-trace's UTF-8 byte cutoff exactly and reject any other mismatch.
export function traceCodeEvidence(code, trace) {
  if (trace.code === code) return 'full'
  const bytes = Buffer.from(code)
  const limit = 64 * 1024
  if (bytes.length <= limit) return undefined
  let end = limit - 24
  while ((bytes[end] & 0xc0) === 0x80) end--
  const preview = `${bytes.toString('utf8', 0, end)}…[+${bytes.length - end} bytes]`
  if (trace.code === preview) return 'truncated'
}

export async function runNeo({
  file,
  row,
  env,
  client,
  output,
  requestId,
  taskId,
  timeout = 135000,
  signal,
}) {
  const code = readFileSync(file, 'utf8'),
    programSha256 = sha(code)
  let record, raw, runId, trace, traceCode, traceVerified
  try {
    raw = await client.callTool(
      {
        name: 'run',
        arguments: { code, timeout },
        _meta: { 'com.browseros.neo/run': { requestId, taskId } },
      },
      undefined,
      { timeout: timeout + 30000, signal },
    )
    runId = raw?._meta?.['com.browseros.neo/run']?.id
    record = raw.structuredContent?.value
    if (
      raw.isError ||
      !record ||
      typeof record !== 'object' ||
      record.harnessError
    )
      record = {
        transport: {
          error:
            record?.harnessError ||
            JSON.stringify(raw.structuredContent?.error) ||
            'no returned record',
        },
      }
    if (runId)
      trace = await hostGet(
        env,
        `/v1/runs/${encodeURIComponent(runId)}?wait=30`,
        { signal },
      )
    if (!trace?.complete) {
      record = { transport: { error: 'missing/incomplete run trace' } }
    } else if (
      trace.id !== runId ||
      trace.requestId !== requestId ||
      trace.taskId !== taskId
    ) {
      record = { transport: { error: 'run trace correlation mismatch' } }
    } else {
      traceCode = traceCodeEvidence(code, trace)
      traceVerified = !!traceCode
      if (!traceVerified)
        record = {
          transport: {
            error: 'stored run source differs from submitted program',
          },
        }
    }
  } catch (error) {
    record = { transport: { error: String(error) } }
  }
  record.id ??= row.id
  record.refusals = (trace?.operations || [])
    .filter((op) => op.error)
    .map((op) => ({ ...op.error, api: op.args?.call?.api || op.name }))
  record.provenance = {
    side: 'neo',
    requestId,
    programSha256,
    browserVersion: env.browserVersion,
    neo: {
      runId,
      replyStatus: raw?._meta?.['com.browseros.neo/run']?.status,
      binarySha256: env.binarySha256,
      gitSha: env.gitSha,
      opCount: trace?.operationCount,
      traceCode,
      traceVerified: traceVerified === true,
    },
  }
  writeFileSync(output, JSON.stringify(record, null, 2))
  writeFileSync(
    output.replace(/\.json$/, '.trace.json'),
    JSON.stringify(trace || null),
  )
  return record
}
