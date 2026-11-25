/**
 * One run's spans, folded into the two shapes the page reads: Neo's run record (`NeoRunRecord`,
 * the same type `GET /v1/runs/{id}` gives, so every reader of `run.neo` works unchanged) and the
 * waterfall (`RunTrace`). Pure and deterministic: folding more spans of a run gives a superset
 * (an operation never disappears and none appears twice), which is what keeps the op table from
 * flickering as spans arrive.
 *
 * Spans carry no arguments, results, notes, kind, effect or screenshots; the record says where it
 * came from (`source: 'spans'`) instead of pretending to have them.
 */

import type {
  NeoOperation,
  NeoOperationError,
  NeoRunRecord,
  RunTrace,
  TraceSpan,
} from '../../shared/api'
import type { NeoSpan, OpSpan, SpanError } from './otlp-json'
import type { RunSpans } from './span-store'

/**
 * The record a run's spans make:
 * - `complete` once `neo.run` has arrived; until then the status is `running`;
 * - times from `neo.run`'s start, or the earliest operation's until it arrives;
 * - operations in `seq` order, parented to the operation they ran inside (null under `neo.run`);
 * - `traceGaps`: seqs missing from 1 to the highest seen (spans the batch processor dropped, or
 *   not yet arrived while the run is incomplete).
 */
export function foldSpans(spans: RunSpans): NeoRunRecord {
  const ops = [...spans.ops.values()].sort((a, b) => a.seq - b.seq)
  const run = spans.run
  const startedAt = run?.startMs ?? earliest(ops)
  const opsBySpan = new Map(ops.map((op) => [op.spanId, op]))
  const operations: NeoOperation[] = ops.map((op) => ({
    id: op.opId,
    seq: op.seq,
    parentId: op.parentSpanId
      ? (opsBySpan.get(op.parentSpanId)?.opId ?? null)
      : null,
    name: op.name,
    kind: '',
    effect: '',
    site: op.site,
    startOffsetMs: ms(op.startMs - startedAt),
    durationMs: op.durationMs,
    tabId: op.tabId,
    args: null,
    argsTruncated: false,
    result: null,
    resultTruncated: false,
    outcome: op.outcome,
    delivery: op.delivery,
    error: operationError(op.error, op.opId),
    notes: [],
    hasScreenshot: false,
  }))
  const highest = ops.at(-1)?.seq ?? 0
  return {
    status: run?.status ?? 'running',
    effects: null,
    startedAt,
    endedAt: run ? run.endMs : null,
    durationMs: run?.durationMs ?? null,
    complete: run !== null,
    tabs: [
      ...new Set(ops.flatMap((op) => (op.tabId === null ? [] : [op.tabId]))),
    ],
    error: run ? operationError(run.error, null) : null,
    operations,
    traceGaps: highest - ops.length,
    source: 'spans',
  }
}

/**
 * The waterfall: `neo.run` at depth 0, its operations under it, an operation that ran inside
 * another one level deeper; depth first, siblings by start time.
 */
export function buildTrace(spans: RunSpans): RunTrace | null {
  const all: NeoSpan[] = [
    ...(spans.run ? [spans.run] : []),
    ...[...spans.ops.values()],
  ]
  const first = all[0]
  if (!first) return null
  const start = Math.min(...all.map((span) => span.startMs))
  const end = Math.max(...all.map((span) => span.endMs))
  const known = new Set(all.map((span) => span.spanId))
  const children = new Map<string | null, NeoSpan[]>()
  for (const span of all) {
    const parent =
      span.parentSpanId && known.has(span.parentSpanId)
        ? span.parentSpanId
        : null
    const siblings = children.get(parent)
    if (siblings) siblings.push(span)
    else children.set(parent, [span])
  }
  const ordered: TraceSpan[] = []
  const visit = (span: NeoSpan, depth: number) => {
    ordered.push(traceSpan(span, depth, start))
    for (const child of sorted(children.get(span.spanId) ?? []))
      visit(child, depth + 1)
  }
  // A root operation's `neo.run` has not arrived yet: it still sits one level in.
  for (const root of sorted(children.get(null) ?? []))
    visit(root, root.kind === 'op' && root.parentSpanId ? 1 : 0)
  return {
    traceId: (spans.run ?? first).traceId,
    complete: spans.run !== null,
    durationMs: ms(end - start),
    spans: ordered,
    resource: first.resource,
  }
}

function traceSpan(span: NeoSpan, depth: number, start: number): TraceSpan {
  return {
    spanId: span.spanId,
    parentSpanId: span.parentSpanId,
    type: span.kind,
    name: span.spanName,
    label: span.kind === 'run' ? span.neoRunId : span.name || span.opId,
    opId: span.kind === 'op' ? span.opId : null,
    startOffsetMs: ms(span.startMs - start),
    durationMs: span.durationMs,
    depth,
    // Only an operation that failed, read from its outcome: an earlier Neo marked a cancelled
    // op's span ERROR too.
    error: span.kind === 'op' && span.outcome === 'error',
    attributes: span.attributes,
  }
}

/** Siblings by start time, then (operations) by seq, so equal starts keep Neo's order. */
function sorted(spans: NeoSpan[]): NeoSpan[] {
  return [...spans].sort(
    (a, b) =>
      a.startMs - b.startMs ||
      (a.kind === 'op' && b.kind === 'op' ? a.seq - b.seq : 0),
  )
}

/** A span's error as Neo's own error shape; spans do not say whether a retry could succeed. */
function operationError(
  error: SpanError | null,
  operationId: string | null,
): NeoOperationError | null {
  if (!error) return null
  return {
    owner: error.owner,
    code: error.code,
    message: error.message,
    retryable: false,
    ...(operationId ? { operationId } : {}),
  }
}

function earliest(ops: OpSpan[]): number {
  return ops.length === 0 ? 0 : Math.min(...ops.map((op) => op.startMs))
}

/** Milliseconds to the microsecond, as spans measure them. */
function ms(value: number): number {
  return Math.round(value * 1000) / 1000
}
