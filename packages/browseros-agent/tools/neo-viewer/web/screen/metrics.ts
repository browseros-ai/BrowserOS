/**
 * The numbers behind the Health view (`HealthView`): a duration histogram's per-bucket counts and
 * their bounds in words, quantiles as the screen writes times, and the raw metric families as
 * lines in Prometheus' own text form, for anyone who wants the exact figures. Pure, read from the
 * API's `MetricsView` as served; bucket bounds come from the data (Neo picks them), never from here.
 */

import type { MetricFamily, Quantiles } from '../../shared/api'
import type { Line } from './text-lines'
import { seconds } from './units'

/** One bucket of a histogram: its upper bound (null: +Inf) and what fell in it alone. */
export interface Bucket {
  le: number | null
  count: number
}

/** Cumulative counts (Prometheus' `le` buckets) to each bucket's own count. */
export function bucketCounts(quantiles: Quantiles): Bucket[] {
  return quantiles.buckets.map((bucket, index) => ({
    le: bucket.le,
    count: Math.max(
      0,
      bucket.count - (quantiles.buckets[index - 1]?.count ?? 0),
    ),
  }))
}

/** A bucket bound as a person reads it: `5 ms`, `2.5 s`, `10 min`, `1 h`; `+Inf` for the last. */
export function boundText(le: number | null): string {
  if (le === null) return '+Inf'
  if (le < 1) return `${trim(le * 1000)} ms`
  if (le < 60) return `${trim(le)} s`
  if (le < 3600) return `${trim(le / 60)} min`
  return `${trim(le / 3600)} h`
}

function trim(value: number): string {
  return String(Number(value.toPrecision(3)))
}

/**
 * A histogram's p50 or p95 as the screen writes times (`168 ms`, `1.23 s`); `–` with no data. A
 * quantile that falls past Neo's last finite bound is only known to be beyond it: Prometheus'
 * estimate is then that bound itself, so it reads `> 10 s` rather than a precise-looking `10.0 s`.
 */
export function quantileText(quantiles: Quantiles, q: 0.5 | 0.95): string {
  const sec = q === 0.5 ? quantiles.p50Sec : quantiles.p95Sec
  if (sec === null) return '–'
  const total = quantiles.buckets.at(-1)?.count ?? 0
  const lastFinite = quantiles.buckets.findLast((bucket) => bucket.le !== null)
  if (lastFinite && lastFinite.le !== null && lastFinite.count < q * total)
    return `> ${boundText(lastFinite.le)}`
  return seconds(sec * 1000)
}

/** The histogram's bounds in one line, for a tooltip: `≤ 5 ms 0 · ≤ 10 ms 2 · … · > 10 s 0`. */
export function bucketsTitle(quantiles: Quantiles): string {
  const buckets = bucketCounts(quantiles)
  const lastFinite = buckets.findLast((bucket) => bucket.le !== null)?.le
  return buckets
    .map((bucket) =>
      bucket.le === null
        ? `> ${boundText(lastFinite ?? null)} ${bucket.count}`
        : `≤ ${boundText(bucket.le)} ${bucket.count}`,
    )
    .join(' · ')
}

/** The ends of a histogram's axis: its first and last finite bounds. */
export function axisEnds(
  quantiles: Quantiles,
): { first: string; last: string } | null {
  const finite = quantiles.buckets.flatMap((bucket) =>
    bucket.le === null ? [] : [bucket.le],
  )
  const first = finite[0]
  const last = finite.at(-1)
  if (first === undefined || last === undefined) return null
  return { first: boundText(first), last: boundText(last) }
}

/**
 * Every family as Prometheus' text exposition writes it (`# HELP`, `# TYPE`, one sample per line,
 * a histogram's `_bucket`, `_sum` and `_count`), toned as the screen tones code: what a
 * person comparing with `curl …/v1/metrics` expects to read.
 */
export function familyLines(families: MetricFamily[]): Line[] {
  const lines: Line[] = []
  for (const family of families) {
    if (lines.length > 0) lines.push([])
    if (family.help)
      lines.push([
        { text: `# HELP ${family.name} ${family.help}`, tone: 'comment' },
      ])
    lines.push([
      { text: `# TYPE ${family.name} ${family.kind}`, tone: 'comment' },
    ])
    for (const series of family.series) {
      if (family.kind === 'histogram') {
        for (const bucket of series.buckets ?? []) {
          lines.push(
            sample(
              `${family.name}_bucket`,
              {
                ...series.labels,
                le: bucket.le === null ? '+Inf' : String(bucket.le),
              },
              bucket.count,
            ),
          )
        }
        lines.push(sample(`${family.name}_sum`, series.labels, series.sum))
        lines.push(sample(`${family.name}_count`, series.labels, series.value))
      } else {
        lines.push(sample(family.name, series.labels, series.value))
      }
    }
  }
  return lines
}

function sample(
  name: string,
  labels: Record<string, string>,
  value: number | null | undefined,
): Line {
  const line: Line = [{ text: name, tone: 'plain' }]
  const entries = Object.entries(labels)
  if (entries.length > 0) {
    line.push({ text: '{', tone: 'muted' })
    entries.forEach(([key, labelValue], index) => {
      if (index > 0) line.push({ text: ',', tone: 'muted' })
      line.push({ text: key, tone: 'key' })
      line.push({ text: '=', tone: 'muted' })
      line.push({ text: JSON.stringify(labelValue), tone: 'string' })
    })
    line.push({ text: '}', tone: 'muted' })
  }
  line.push({
    text: ` ${value === null || value === undefined ? 'NaN' : String(value)}`,
    tone: 'plain',
  })
  return line
}

/** A by-label map as rows, largest first, then by name: what the tables list. */
export function ranked(byKey: Record<string, number>): Array<[string, number]> {
  return Object.entries(byKey).sort(
    ([a, x], [b, y]) => y - x || a.localeCompare(b),
  )
}
