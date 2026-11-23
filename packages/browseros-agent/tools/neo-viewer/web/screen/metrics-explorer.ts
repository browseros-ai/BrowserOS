/**
 * Duration ranges for the Metrics explorer. Neo sends cumulative histogram counts in seconds;
 * this module subtracts exact bounds and supplies the same exclusive-low, inclusive-high
 * millisecond intervals that the Traces explorer searches. Method outcomes remain separate.
 */
import type { MetricFamily, MetricsView, Quantiles } from '../../shared/api'
import type { ExplorerSpanFilter } from './explorer-filter'
import { boundText } from './metrics'

type Operation = MetricsView['operations'][number]

/** Counters without outcome labels cannot distinguish a success from a cancellation. */
export type OperationOutcomes =
  | { kind: 'labeled'; ok: number; error: number; cancelled: number }
  | { kind: 'unlabeled'; nonFailed: number; failed: number }

export function operationOutcomes(
  families: MetricFamily[],
  operation: Operation,
): OperationOutcomes {
  const samples = families.flatMap((family) =>
    family.name === 'neo_operations_total' && family.kind === 'counter'
      ? family.series.filter((one) => one.labels.neo_op_name === operation.op)
      : [],
  )
  const counts = { ok: 0, error: 0, cancelled: 0 }
  let known = samples.length > 0
  for (const sample of samples) {
    const outcome = sample.labels.neo_op_outcome
    if (
      sample.value === null ||
      (outcome !== 'ok' && outcome !== 'error' && outcome !== 'cancelled')
    ) {
      known = false
      break
    }
    counts[outcome] += sample.value
  }
  return known &&
    counts.ok + counts.error + counts.cancelled === operation.total
    ? { kind: 'labeled', ...counts }
    : {
        kind: 'unlabeled',
        nonFailed: Math.max(0, operation.total - operation.errors),
        failed: operation.errors,
      }
}

/** Null below means the start of the duration domain, and above means no upper limit. */
export interface DurationBand {
  minMs: number | null
  maxMs: number | null
  label: string
}

export const DURATION_BANDS: readonly DurationBand[] = [
  { minMs: null, maxMs: 25, label: '≤25 ms' },
  { minMs: 25, maxMs: 100, label: '>25 to 100 ms' },
  { minMs: 100, maxMs: 500, label: '>100 to 500 ms' },
  { minMs: 500, maxMs: 1000, label: '>500 ms to 1 s' },
  { minMs: 1000, maxMs: null, label: 'Over 1 s' },
]

/** An absent finite bound is unknown. Interpolating it would invent observations. */
export function durationBandCount(
  duration: Pick<Quantiles, 'count' | 'buckets'>,
  band: Pick<DurationBand, 'minMs' | 'maxMs'>,
): number | null {
  if (duration.count === 0) return 0
  const lower = band.minMs === null ? 0 : cumulativeAt(duration, band.minMs)
  const upper =
    band.maxMs === null ? duration.count : cumulativeAt(duration, band.maxMs)
  return lower === null || upper === null || upper < lower
    ? null
    : upper - lower
}

function cumulativeAt(
  duration: Pick<Quantiles, 'buckets'>,
  ms: number,
): number | null {
  return (
    duration.buckets.find((bucket) => bucket.le === ms / 1000)?.count ?? null
  )
}

/**
 * Error counts come from outcome-labeled histogram series, not the method's failure counter.
 * Every series must identify its outcome and represent this exact interval; otherwise a
 * missing status or mismatched bound could make a partial error count look like a known zero.
 */
export function failedDurationBandCount(
  families: MetricFamily[],
  operation: Operation,
  band: Pick<DurationBand, 'minMs' | 'maxMs'>,
): number | null {
  const series = families.flatMap((family) =>
    family.name === 'neo_operation_duration_seconds' &&
    family.kind === 'histogram'
      ? family.series.filter((one) => one.labels.neo_op_name === operation.op)
      : [],
  )
  if (series.length === 0) return null
  let total = 0
  let failed = 0
  for (const one of series) {
    const outcome = one.labels.neo_op_outcome
    if (
      one.value === null ||
      !['ok', 'error', 'cancelled'].includes(outcome ?? '')
    )
      return null
    const inBand = durationBandCount(
      { count: one.value, buckets: one.buckets ?? [] },
      band,
    )
    if (inBand === null) return null
    total += one.value
    if (outcome === 'error') failed += inBand
  }
  return total === operation.duration.count ? failed : null
}

/** A whole-instance range is known only when every measured method supplies its bounds. */
export function operationBandCount(
  operations: Operation[],
  band: Pick<DurationBand, 'minMs' | 'maxMs'>,
): number | null {
  let total = 0
  for (const operation of operations) {
    const value = durationBandCount(operation.duration, band)
    if (value === null) return null
    total += value
  }
  return total
}

/** A measured slow bucket carries a complete trace filter, never a fabricated failure count. */
export interface SlowOperationBucket {
  filter: ExplorerSpanFilter
  count: number
  label: string
}

/**
 * Keep Neo's own disjoint buckets for drilldowns, including custom bounds. A bucket straddling
 * 500 ms cannot tell us how many calls were slow, so only intervals wholly above it are listed.
 * The observation count closes a missing +Inf bucket without assuming a missing finite bound.
 */
export function slowOperationBuckets(
  operations: Operation[],
): SlowOperationBucket[] {
  const rows: SlowOperationBucket[] = []
  for (const operation of operations) {
    let minMs = 0
    let previousCount = 0
    for (const bucket of operation.duration.buckets) {
      const maxMs = bucket.le === null ? null : bucket.le * 1000
      addBucket(operation.op, minMs, maxMs, bucket.count - previousCount)
      if (maxMs === null) break
      minMs = maxMs
      previousCount = bucket.count
    }
    if (
      operation.duration.buckets.length > 0 &&
      operation.duration.buckets.at(-1)?.le !== null
    ) {
      addBucket(
        operation.op,
        minMs,
        null,
        operation.duration.count - previousCount,
      )
    }
  }
  return rows.sort(
    (a, b) =>
      b.count - a.count ||
      a.filter.method.localeCompare(b.filter.method) ||
      a.filter.minMs - b.filter.minMs,
  )

  function addBucket(
    method: string,
    minMs: number,
    maxMs: number | null,
    value: number,
  ) {
    if (minMs < 500 || value <= 0) return
    rows.push({
      filter: { method, minMs, maxMs },
      count: value,
      label:
        maxMs === null
          ? `Over ${boundText(minMs / 1000)}`
          : `>${boundText(minMs / 1000)} to ${boundText(maxMs / 1000)}`,
    })
  }
}
