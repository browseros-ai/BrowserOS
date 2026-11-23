/**
 * Read-only search of the bounded OTLP span store, joined to the current proxy log. It scans
 * retained spans once and builds summaries, never waterfall trees or full run/reply payloads.
 * Ownership comes from Neo run ids or viewer-minted parents, so caller-shared trace ids cannot
 * leak another run's operations into a match and Clear removes rows without deleting telemetry.
 */
import type {
  TraceRunSummary,
  TraceSearchQuery,
  TraceSearchResult,
  TracesResponse,
} from '../../shared/api'
import type { OpSpan } from './otlp-json'
import type { RunSpans } from './span-store'

export type TraceRunLookup = (
  neoRunId: string,
  mintedSpanId: string | null,
) => Iterable<TraceRunSummary>

type TraceSlice = Omit<TraceSearchResult, 'run'>

/** Parse untrusted URL parameters once, before search sees them. */
export function parseTraceQuery(params: URLSearchParams): TraceSearchQuery {
  const traceId = (params.get('traceId') ?? '').trim().toLowerCase()
  const method = (params.get('method') ?? '').trim()
  if (!/^[0-9a-f]{0,32}$/.test(traceId))
    throw new Error('traceId must be at most 32 hex characters')
  if (method.length > 256)
    throw new Error('method must be at most 256 characters')
  const minMs = durationParam(params, 'minMs')
  const maxMs = durationParam(params, 'maxMs')
  if (minMs !== null && maxMs !== null && minMs >= maxMs)
    throw new Error('maxMs must be greater than minMs')
  const rawLimit = params.get('limit')
  const limit = rawLimit === null ? 100 : Number(rawLimit)
  if (!Number.isInteger(limit) || limit < 1 || limit > 200)
    throw new Error('limit must be an integer from 1 to 200')
  const cursor = cursorParam(params.get('cursor'))
  return { traceId, method, minMs, maxMs, limit, cursor }
}

/** A cursor names an arrival, not a mutable span start or a log's renumbered run sequence. */
function cursorParam(raw: string | null): TraceSearchQuery['cursor'] {
  if (raw === null) return null
  if (/^[A-Za-z0-9_-]{1,1024}$/.test(raw)) {
    try {
      const parsed: unknown = JSON.parse(
        Buffer.from(raw, 'base64url').toString(),
      )
      if (
        Array.isArray(parsed) &&
        parsed.length === 2 &&
        typeof parsed[0] === 'string' &&
        parsed[0].length <= 40 &&
        Number.isFinite(Date.parse(parsed[0])) &&
        typeof parsed[1] === 'string' &&
        parsed[1].length > 0 &&
        parsed[1].length <= 256
      )
        return { sentAt: parsed[0], id: parsed[1] }
    } catch {
      // Malformed JSON is a bad request just like an invalid cursor shape below.
    }
  }
  throw new Error('cursor must be a trace-search continuation token')
}

function durationParam(params: URLSearchParams, name: string): number | null {
  const raw = params.get(name)?.trim()
  if (!raw) return null
  const value = Number(raw)
  if (!Number.isFinite(value) || value < 0)
    throw new Error(
      `${name} must be a finite non-negative number of milliseconds`,
    )
  return value
}

export function searchTraces(
  retained: Iterable<RunSpans>,
  query: TraceSearchQuery,
  lookup: TraceRunLookup,
): TracesResponse {
  // Arrival timestamps and viewer ids never change when a late root span or reply lands.
  // Keep only one page plus lookahead, even if a retained Neo run has several proxy bindings.
  const page: TraceSearchResult[] = []
  for (const spans of retained) {
    const summary = summarizeTrace(spans, query)
    if (!summary) continue
    for (const run of lookup(spans.neoRunId, spans.run?.parentSpanId ?? null)) {
      if (query.cursor && arrivalOrder(run, query.cursor) <= 0) continue
      page.push({ ...summary, run })
      page.sort((a, b) => arrivalOrder(a.run, b.run))
      if (page.length > query.limit + 1) page.pop()
    }
  }
  const truncated = page.length > query.limit
  const traces = page.slice(0, query.limit)
  const last = traces.at(-1)
  return {
    traces,
    truncated,
    nextCursor:
      truncated && last
        ? Buffer.from(JSON.stringify([last.run.sentAt, last.run.id])).toString(
            'base64url',
          )
        : null,
  }
}

function arrivalOrder(
  a: Pick<TraceRunSummary, 'sentAt' | 'id'>,
  b: Pick<TraceRunSummary, 'sentAt' | 'id'>,
): number {
  return Date.parse(b.sentAt) - Date.parse(a.sentAt) || a.id.localeCompare(b.id)
}

/** A cheap summary of retained spans; matching never folds records or sorts a waterfall. */
function summarizeTrace(
  spans: RunSpans,
  query: TraceSearchQuery,
): TraceSlice | null {
  const filtered =
    query.method !== '' || query.minMs !== null || query.maxMs !== null
  const first = spans.run ?? spans.ops.values().next().value
  if (!first?.traceId.includes(query.traceId)) return null
  let start = first.startMs
  let end = first.endMs
  let errorCount = 0
  let match: OpSpan | null = null
  for (const op of spans.ops.values()) {
    start = Math.min(start, op.startMs)
    end = Math.max(end, op.endMs)
    if (op.outcome === 'error') errorCount++
    if (!filtered || !operationMatches(op, query)) continue
    if (
      !match ||
      op.startMs < match.startMs ||
      (op.startMs === match.startMs && op.seq < match.seq)
    )
      match = op
  }
  if (filtered && !match) return null
  return {
    traceId: first.traceId,
    startedAt: start,
    durationMs: end - start,
    spanCount: spans.ops.size + (spans.run ? 1 : 0),
    errorCount,
    matchedSpan: match
      ? {
          spanId: match.spanId,
          name: match.spanName,
          label: match.name,
          durationMs: match.durationMs,
        }
      : null,
  }
}

/** Every predicate describes this one operation, never timings from another span in the run. */
function operationMatches(op: OpSpan, query: TraceSearchQuery): boolean {
  const duration = op.durationMs
  return (
    (!query.method || op.spanName === query.method) &&
    (query.minMs === null || duration > query.minMs) &&
    (query.maxMs === null || duration <= query.maxMs)
  )
}
