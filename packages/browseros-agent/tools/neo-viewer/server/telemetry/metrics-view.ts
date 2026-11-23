/**
 * What the viewer reads out of Neo's 16 metric families (wire-contract.md "Metrics"): the few
 * numbers health is made of (`totalsOf`) and the Health view's tables (`metricsView`). Pure, and
 * the same for a scrape or a push, since both give the one `MetricFamily` model. A family that is
 * missing (an older image, or an instrument that has recorded nothing yet) counts as zero.
 */

import type {
  MetricFamily,
  MetricsView,
  QuantileReading,
  Quantiles,
} from '../../shared/api'

type Series = MetricFamily['series'][number]

/** Cumulative counters and live gauges in one sample. */
export interface Totals {
  runs: number
  runErrors: number
  ops: number
  opErrors: number
  crashes: number
  runsInFlight: number
  tabsOpen: number
}

export function totalsOf(families: MetricFamily[]): Totals {
  const runs = seriesOf(families, 'neo_runs_total')
  const ops = seriesOf(families, 'neo_operations_total')
  return {
    runs: sum(runs),
    runErrors: sum(runs.filter(failed)),
    ops: sum(ops),
    opErrors: sum(ops.filter(failed)),
    crashes: sum(seriesOf(families, 'neo_renderer_crashes_total')),
    runsInFlight: sum(seriesOf(families, 'neo_run_active')),
    tabsOpen: sum(seriesOf(families, 'neo_tab_count')),
  }
}

export function metricsView(
  families: MetricFamily[],
  source: MetricsView['source'],
  observedAt: number,
): MetricsView {
  const totals = totalsOf(families)
  const opSeries = seriesOf(families, 'neo_operations_total')
  const opDurations = seriesOf(families, 'neo_operation_duration_seconds')
  const operations = [...groupBy(opSeries, 'neo_op_name')].map(
    ([op, series]) => ({
      op,
      total: sum(series),
      errors: sum(series.filter(failed)),
      duration: quantiles(
        opDurations.filter((candidate) => candidate.labels.neo_op_name === op),
      ),
    }),
  )
  operations.sort((a, b) => b.total - a.total || a.op.localeCompare(b.op))
  const navigations = seriesOf(families, 'neo_navigations_total')
  const solves = seriesOf(families, 'neo_captcha_solves_total')
  const errors = seriesOf(families, 'neo_errors_total')
    .map((series) => ({
      owner: series.labels.neo_error_owner ?? '',
      code: series.labels.error_type ?? '',
      count: series.value ?? 0,
    }))
    .filter((entry) => entry.count > 0)
    .sort((a, b) => b.count - a.count || a.code.localeCompare(b.code))
  return {
    source,
    observedAt,
    runs: {
      total: totals.runs,
      inFlight: totals.runsInFlight,
      byStatus: sumBy(seriesOf(families, 'neo_runs_total'), 'neo_run_status'),
      duration: quantiles(seriesOf(families, 'neo_run_duration_seconds')),
    },
    operations,
    errors,
    tabs: {
      openByOrigin: sumBy(
        seriesOf(families, 'neo_tab_count'),
        'neo_tab_origin',
      ),
      openedByOrigin: sumBy(
        seriesOf(families, 'neo_tab_opened_total'),
        'neo_tab_origin',
      ),
      closedByReason: sumBy(
        seriesOf(families, 'neo_tab_closed_total'),
        'neo_tab_close_reason',
      ),
      lifetime: quantiles(seriesOf(families, 'neo_tab_duration_seconds')),
    },
    crashes: totals.crashes,
    navigations: {
      total: sum(navigations),
      byStatus: sumBy(navigations, 'http_response_status_code', 'none'),
      errors: sumBy(
        seriesOf(families, 'neo_navigation_errors_total'),
        'error_type',
      ),
    },
    captchas: {
      total: sum(solves),
      solves: solves
        .map((series) => ({
          family: series.labels.neo_captcha_family ?? '',
          verdict: series.labels.neo_captcha_verdict ?? '',
          count: series.value ?? 0,
        }))
        .filter((entry) => entry.count > 0)
        .sort(
          (a, b) =>
            b.count - a.count ||
            a.family.localeCompare(b.family) ||
            a.verdict.localeCompare(b.verdict),
        ),
      byReason: sumBy(
        seriesOf(families, 'neo_captcha_reasons_total'),
        'neo_captcha_reason',
      ),
      spendUsd: sumBy(
        seriesOf(families, 'neo_captcha_spend_total'),
        'neo_captcha_provider',
      ),
    },
    families,
  }
}

/**
 * Histogram series summed into one (buckets at every bound any of them has, each series counted
 * at its largest bound at or below it), with p50 and p95 read off as Prometheus'
 * `histogram_quantile` does.
 */
export function quantiles(series: Series[]): Quantiles {
  const bounds = [
    ...new Set(
      series.flatMap((one) =>
        (one.buckets ?? []).flatMap((bucket) =>
          bucket.le === null ? [] : [bucket.le],
        ),
      ),
    ),
  ].sort((a, b) => a - b)
  const atBound = (one: Series, le: number | null) => {
    let count = 0
    for (const bucket of one.buckets ?? []) {
      if (
        le === null ? bucket.le === null : bucket.le !== null && bucket.le <= le
      )
        count = Math.max(count, bucket.count)
    }
    return count
  }
  const buckets = [...bounds, null].map((le) => ({
    le,
    count: series.reduce((total, one) => total + atBound(one, le), 0),
  }))
  const count = series.reduce((total, one) => total + (one.value ?? 0), 0)
  const p50 = histogramQuantile(0.5, buckets)
  const p95 = histogramQuantile(0.95, buckets)
  return {
    count,
    p50Sec: p50.sec,
    p95Sec: p95.sec,
    p50Is: p50.is,
    p95Is: p95.is,
    buckets,
  }
}

/**
 * The q-quantile of cumulative buckets, never more than the data supports. Inside a bucket that
 * slower observations lie above, it is interpolated linearly as Prometheus' `histogram_quantile`
 * does (the first bucket from 0): that cannot pass the slowest observation. In the bucket that
 * holds the slowest observations it can (a 62 s run in the 60–120 s bucket would read 81 s), so
 * there it is that bucket's upper bound, read "at most"; past the largest finite bound, that bound,
 * read "over". Null with no observations.
 */
export function histogramQuantile(
  q: number,
  buckets: Array<{ le: number | null; count: number }>,
): { sec: number | null; is: QuantileReading } {
  const total = buckets.at(-1)?.count ?? 0
  if (total <= 0) return { sec: null, is: 'estimate' }
  const rank = q * total
  // The bucket the slowest observations are in: the last one whose count rises.
  let slowest = 0
  buckets.forEach((bucket, index) => {
    if (bucket.count > (buckets[index - 1]?.count ?? 0)) slowest = index
  })
  let lowerBound = 0
  let lowerCount = 0
  for (const [index, bucket] of buckets.entries()) {
    if (bucket.count >= rank) {
      if (bucket.le === null) return { sec: lowerBound, is: 'over' }
      if (index >= slowest) return { sec: bucket.le, is: 'atMost' }
      const inBucket = bucket.count - lowerCount
      return {
        sec:
          inBucket <= 0
            ? bucket.le
            : lowerBound +
              (bucket.le - lowerBound) * ((rank - lowerCount) / inBucket),
        is: 'estimate',
      }
    }
    if (bucket.le !== null) lowerBound = bucket.le
    lowerCount = bucket.count
  }
  return { sec: lowerBound, is: 'over' }
}

function seriesOf(families: MetricFamily[], name: string): Series[] {
  return families.find((family) => family.name === name)?.series ?? []
}

/**
 * A run or operation someone has to act on: Neo attributed it (`neo_error_owner` other than
 * `none`); without that label, any run status or op outcome but `ok`.
 */
function failed(series: Series): boolean {
  const owner = series.labels.neo_error_owner
  if (owner !== undefined) return owner !== 'none'
  return (series.labels.neo_run_status ?? series.labels.neo_op_outcome) !== 'ok'
}

function sum(series: Series[]): number {
  return series.reduce((total, one) => total + (one.value ?? 0), 0)
}

/** `series` summed by `label`; a series without it is filed under `missing`. */
function sumBy(
  series: Series[],
  label: string,
  missing = '',
): Record<string, number> {
  const out: Record<string, number> = {}
  for (const [key, members] of groupBy(series, label, missing))
    out[key] = sum(members)
  return out
}

function groupBy(
  series: Series[],
  label: string,
  missing = '',
): Map<string, Series[]> {
  const groups = new Map<string, Series[]>()
  for (const one of series) {
    const key = one.labels[label] ?? missing
    const group = groups.get(key)
    if (group) group.push(one)
    else groups.set(key, [one])
  }
  return groups
}
