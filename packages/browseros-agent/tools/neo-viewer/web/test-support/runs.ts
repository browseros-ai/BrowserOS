/** Run records for the screen's tests, shaped as the API sends them. */

import type { RunDetail, RunSummary } from '../../shared/api'
import { textSize } from '../../shared/api'

const T0 = Date.parse('2026-11-11T10:14:02.100Z')

/**
 * A finished `ok` run: sent `at` ms after T0, answered `roundTripMs` later, held `proxyMs` by the
 * viewer. `reply` is the text the model read.
 */
export function runSummary(
  seq: number,
  options: {
    at?: number
    roundTripMs?: number | null
    proxyMs?: number
    gapMs?: number | null
    code?: string
    reply?: string | null
    status?: RunSummary['status']
  } = {},
): RunSummary {
  const at = options.at ?? seq * 10_000
  const roundTripMs =
    options.roundTripMs === undefined ? 100 : options.roundTripMs
  const status = options.status ?? (roundTripMs === null ? 'running' : 'ok')
  const code = options.code ?? `return ${seq}`
  const reply =
    options.reply === undefined
      ? status === 'running'
        ? null
        : String(seq)
      : options.reply
  return {
    id: `run-${seq}`,
    taskId: 'task-1',
    seq,
    tool: 'run',
    status,
    code,
    timeoutMs: null,
    taskIdStamped: true,
    sentAt: new Date(T0 + at).toISOString(),
    answeredAt:
      roundTripMs === null
        ? null
        : new Date(T0 + at + roundTripMs).toISOString(),
    roundTripMs,
    proxyMs: options.proxyMs ?? 0,
    gapMs: options.gapMs === undefined ? null : options.gapMs,
    codeSize: textSize(code),
    replySize: reply === null ? null : textSize(reply),
    replyPreview: reply === null ? null : reply.slice(0, 200),
    neoRunId: reply === null ? null : `neo-${seq}`,
    neoStatus: status === 'running' ? null : status,
    neoEffects: null,
    neoDurationMs: null,
    operationCount: null,
  }
}

/** The run in full, as `GET /api/runs/:id` answers; `text` defaults to the summary's preview. */
export function runDetail(
  run: RunSummary,
  text: string | null = run.replyPreview,
): RunDetail {
  return {
    ...run,
    neo: null,
    request: { raw: '{}', forwarded: null, jsonRpcId: run.seq },
    reply: {
      httpStatus: text === null ? null : 200,
      contentType: 'application/json',
      contentEncoding: null,
      raw: text,
      text,
      structuredContent: null,
      isError: run.status === 'error',
      meta: null,
      rpcError: null,
      notifications: [],
      keepAlives: 0,
    },
  }
}
