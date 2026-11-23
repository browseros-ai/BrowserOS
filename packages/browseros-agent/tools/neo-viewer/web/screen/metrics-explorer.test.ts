import { describe, expect, it } from 'bun:test'
import { metricsView as buildMetricsView } from '../../server/telemetry/metrics-view'
import type { MetricFamily, MetricsView, Quantiles } from '../../shared/api'
import { quantiles } from '../test-support/telemetry'
import {
  DURATION_BANDS,
  durationBandCount,
  failedDurationBandCount,
  operationBandCount,
  operationOutcomes,
  slowOperationBuckets,
} from './metrics-explorer'

const operation = (duration: Quantiles): MetricsView['operations'][number] => ({
  op: 'locator.click',
  total: duration.count,
  errors: 1,
  duration,
})

function series(
  outcome: string | undefined,
  observations: number[],
): MetricFamily['series'][number] {
  const duration = quantiles(observations, null, null)
  return {
    labels: {
      neo_op_name: 'locator.click',
      ...(outcome === undefined ? {} : { neo_op_outcome: outcome }),
    },
    value: duration.count,
    buckets: duration.buckets,
  }
}

function family(samples: MetricFamily['series']): MetricFamily[] {
  return [
    {
      name: 'neo_operation_duration_seconds',
      kind: 'histogram',
      help: null,
      series: samples,
    },
  ]
}

describe('Metrics explorer duration ranges', () => {
  it('keeps cancellations distinct from successes when the outcome labels are available', () => {
    const op = operation(quantiles([0.6, 0.7, 0.8], null, null))
    const counters: MetricFamily[] = [
      {
        name: 'neo_operations_total',
        kind: 'counter',
        help: null,
        series: [
          series('ok', [0.6]),
          series('error', [0.7]),
          series('cancelled', [0.8]),
        ],
      },
    ]
    expect(operationOutcomes(counters, op)).toEqual({
      kind: 'labeled',
      ok: 1,
      error: 1,
      cancelled: 1,
    })
    expect(operationOutcomes([], op)).toEqual({
      kind: 'unlabeled',
      nonFailed: 2,
      failed: 1,
    })
  })
  it('subtracts cumulative buckets into disjoint inclusive-upper ranges', () => {
    const duration = quantiles([0, 0.025, 0.1, 0.5, 0.5001, 1, 1.1], null, null)
    expect(
      DURATION_BANDS.map((band) => durationBandCount(duration, band)),
    ).toEqual([2, 1, 1, 2, 1])
    expect(
      operationBandCount([operation(duration)], { minMs: 500, maxMs: null }),
    ).toBe(3)
  })

  it('does not interpolate a missing bound or report partial method totals as complete', () => {
    const complete = operation(quantiles([0.6], null, null))
    const missing = operation({
      ...quantiles([0.6], null, null),
      buckets: [
        { le: 1, count: 1 },
        { le: null, count: 1 },
      ],
    })
    expect(
      durationBandCount(missing.duration, { minMs: 500, maxMs: 1000 }),
    ).toBeNull()
    expect(
      operationBandCount([complete, missing], { minMs: 500, maxMs: null }),
    ).toBeNull()
    expect(
      durationBandCount(missing.duration, { minMs: 1000, maxMs: null }),
    ).toBe(0)
  })

  it('requires raw bounds in every outcome and drills down only through their intersection', () => {
    const samples = family([
      {
        labels: { neo_op_name: 'locator.click', neo_op_outcome: 'ok' },
        value: 5,
        buckets: [
          { le: 0.5, count: 1 },
          { le: 1, count: 3 },
          { le: 2.5, count: 4 },
          { le: null, count: 5 },
        ],
      },
      {
        labels: { neo_op_name: 'locator.click', neo_op_outcome: 'error' },
        value: 3,
        buckets: [
          { le: 0.5, count: 1 },
          { le: 2.5, count: 2 },
          { le: null, count: 3 },
        ],
      },
    ])
    samples.push({
      name: 'neo_operations_total',
      kind: 'counter',
      help: null,
      series: [{ labels: { neo_op_name: 'locator.click' }, value: 8 }],
    })
    const view = buildMetricsView(samples, 'push', 1)
    expect(
      operationBandCount(view.operations, { minMs: 500, maxMs: 1000 }, samples),
    ).toBeNull()
    expect(
      operationBandCount(view.operations, { minMs: 500, maxMs: null }, samples),
    ).toBe(6)
    expect(slowOperationBuckets(view.operations, samples)).toEqual([
      {
        filter: { method: 'locator.click', minMs: 500, maxMs: 2500 },
        count: 4,
        label: '>500 ms to 2.5 s',
      },
      {
        filter: { method: 'locator.click', minMs: 2500, maxMs: null },
        count: 2,
        label: 'Over 2.5 s',
      },
    ])
  })

  it('treats an empty histogram as no observations rather than unavailable bounds', () => {
    const empty = { ...quantiles([], null, null), buckets: [] }
    expect(
      DURATION_BANDS.map((band) => durationBandCount(empty, band)),
    ).toEqual([0, 0, 0, 0, 0])
    expect(slowOperationBuckets([operation(empty)])).toEqual([])
    const unbounded = { ...empty, count: 4 }
    expect(durationBandCount(unbounded, { minMs: 500, maxMs: null })).toBeNull()
    expect(slowOperationBuckets([operation(unbounded)])).toEqual([])
  })

  it('keeps incomplete raw histograms unknown instead of using the merged summary', () => {
    const op = operation(quantiles([0.6, 0.7], null, null))
    for (const samples of [
      family([series('ok', [0.6])]),
      family([{ ...series('ok', [0.6, 0.7]), value: null }]),
    ]) {
      expect(
        operationBandCount([op], { minMs: 500, maxMs: null }, samples),
      ).toBeNull()
      expect(slowOperationBuckets([op], samples)).toEqual([])
    }
  })

  it('does not require finite bounds from zero-observation raw series', () => {
    const op = operation(quantiles([0.6], null, null))
    const samples = family([
      series('ok', [0.6]),
      { labels: { neo_op_name: op.op, neo_op_outcome: 'error' }, value: 0 },
    ])
    expect(operationBandCount([op], { minMs: 500, maxMs: 1000 }, samples)).toBe(
      1,
    )
    expect(slowOperationBuckets([op], samples)).toEqual([
      {
        filter: { method: op.op, minMs: 500, maxMs: 1000 },
        count: 1,
        label: '>500 ms to 1 s',
      },
    ])
  })

  it('uses custom bucket bounds for exact trace filters and omits a bucket crossing 500 ms', () => {
    const duration = {
      ...quantiles([], null, null),
      count: 6,
      buckets: [
        { le: 0.8, count: 3 },
        { le: 1.7, count: 5 },
        { le: null, count: 6 },
      ],
    }
    expect(slowOperationBuckets([operation(duration)])).toEqual([
      {
        filter: { method: 'locator.click', minMs: 800, maxMs: 1700 },
        count: 2,
        label: '>800 ms to 1.7 s',
      },
      {
        filter: { method: 'locator.click', minMs: 1700, maxMs: null },
        count: 1,
        label: 'Over 1.7 s',
      },
    ])
  })

  it('counts only exact error-labeled duration samples in each interval', () => {
    const op = operation(quantiles([0.02, 0.6, 0.7, 1.2, 5], null, null))
    op.errors = 4
    const samples = family([
      series('error', [0.02, 0.6, 1.2]),
      series('ok', [0.7]),
      series('cancelled', [5]),
    ])
    expect(
      failedDurationBandCount(samples, op, { minMs: 500, maxMs: 1000 }),
    ).toBe(1)
    expect(
      failedDurationBandCount(samples, op, { minMs: 1000, maxMs: null }),
    ).toBe(1)
    expect(
      failedDurationBandCount(samples, op, { minMs: 2500, maxMs: 5000 }),
    ).toBe(0)
  })

  it('shows a known zero with labeled successful samples but no error series', () => {
    const op = operation(quantiles([0.6], null, null))
    expect(
      failedDurationBandCount(family([series('ok', [0.6])]), op, {
        minMs: 500,
        maxMs: 1000,
      }),
    ).toBe(0)
  })

  it('keeps bucket failures unknown with missing outcomes, misaligned bounds or partial series', () => {
    const op = operation(quantiles([0.6, 0.7], null, null))
    const band = { minMs: 500, maxMs: 1000 }
    expect(failedDurationBandCount([], op, band)).toBeNull()
    expect(
      failedDurationBandCount(
        family([series(undefined, [0.6, 0.7])]),
        op,
        band,
      ),
    ).toBeNull()
    const misaligned = series('error', [0.6])
    misaligned.buckets = misaligned.buckets?.filter(
      (bucket) => bucket.le !== 0.5,
    )
    expect(
      failedDurationBandCount(
        family([series('ok', [0.7]), misaligned]),
        op,
        band,
      ),
    ).toBeNull()
    expect(
      failedDurationBandCount(family([series('ok', [0.6])]), op, band),
    ).toBeNull()
  })

  it('uses the observation count for a missing open-ended bucket', () => {
    const duration = {
      ...quantiles([], null, null),
      count: 5,
      buckets: [
        { le: 0.5, count: 2 },
        { le: 1, count: 3 },
      ],
    }
    expect(slowOperationBuckets([operation(duration)])).toEqual([
      {
        filter: { method: 'locator.click', minMs: 1000, maxMs: null },
        count: 2,
        label: 'Over 1 s',
      },
      {
        filter: { method: 'locator.click', minMs: 500, maxMs: 1000 },
        count: 1,
        label: '>500 ms to 1 s',
      },
    ])
  })
})
