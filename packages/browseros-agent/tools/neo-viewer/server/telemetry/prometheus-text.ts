/**
 * Prometheus text exposition (format 0.0.4, what Neo's `GET /v1/metrics` serves) into the
 * viewer's `MetricFamily` model, the one the OTLP decoder produces too. Pure. It also reads what
 * OpenMetrics adds, so an exporter that speaks it does not break the panel: `# EOF`, `# UNIT`,
 * counters declared without `_total`, `_created` samples, exemplars after `#`, quoted names.
 *
 * A line it cannot read is skipped, never fatal: one odd line must not blank the Health view.
 * Histogram buckets come from the data; no bound is assumed (wire-contract.md).
 */

import type { MetricFamily } from '../../shared/api'

/** A declared type, and the sample-name suffixes that belong to a family of it. */
const SUFFIXES: Record<string, string[]> = {
  counter: ['_total', '_created'],
  histogram: ['_bucket', '_count', '_sum', '_created'],
  gaugehistogram: ['_bucket', '_gcount', '_gsum'],
  summary: ['_count', '_sum', '_created'],
  info: ['_info'],
}

const NAME = /^[a-zA-Z_:][a-zA-Z0-9_:]*/

interface Sample {
  name: string
  labels: Record<string, string>
  value: number
}

interface Declared {
  type: string
  help: string | null
}

export function parsePrometheus(text: string): MetricFamily[] {
  const declared = new Map<string, Declared>()
  const samples: Sample[] = []
  for (const rawLine of text.split('\n')) {
    const line = rawLine.trim()
    if (line === '') continue
    if (line.startsWith('#')) {
      if (/^#\s*EOF\s*$/.test(line)) break
      readComment(line, declared)
      continue
    }
    const sample = readSample(line)
    if (sample) samples.push(sample)
  }
  return build(declared, samples)
}

/** `# HELP name text` and `# TYPE name type`; `# UNIT` and other comments are skipped. */
function readComment(line: string, declared: Map<string, Declared>): void {
  const match = /^#\s+(HELP|TYPE)\s+(\S+)(?:\s+(.*))?$/.exec(line)
  if (!match) return
  const [, keyword, name = '', rest = ''] = match
  const entry = declared.get(name) ?? { type: 'untyped', help: null }
  if (keyword === 'TYPE') entry.type = rest.trim().toLowerCase()
  else entry.help = unescapeText(rest, false)
  declared.set(name, entry)
}

/** `name{label="value",…} value [timestamp] [# exemplar]`. */
function readSample(line: string): Sample | null {
  let name = NAME.exec(line)?.[0] ?? ''
  let at = name.length
  const labels: Record<string, string> = {}
  if (line[at] === '{') {
    const read = readLabels(line, at + 1, labels)
    if (read === null) return null
    if (read.name) name = read.name
    at = read.end
  }
  if (name === '') return null
  // After the labels a `#` can only start an exemplar: a value or a timestamp has none.
  const rest = line.slice(at).split('#')[0]?.trim() ?? ''
  const value = parseValue(rest.split(/\s+/)[0] ?? '')
  return value === undefined ? null : { name, labels, value }
}

/**
 * The label set from just after `{`; returns where it ended, and the metric's name when the set
 * carries it as a bare quoted string (OpenMetrics 2 / Prometheus 3 UTF-8 names).
 */
function readLabels(
  line: string,
  start: number,
  labels: Record<string, string>,
): { end: number; name: string | null } | null {
  let at = start
  let name: string | null = null
  for (;;) {
    while (line[at] === ' ' || line[at] === '\t' || line[at] === ',') at++
    if (line[at] === '}') return { end: at + 1, name }
    let key: string
    if (line[at] === '"') {
      const quoted = readQuoted(line, at)
      if (!quoted) return null
      key = quoted.text
      at = quoted.end
    } else {
      key = NAME.exec(line.slice(at))?.[0] ?? ''
      if (key === '') return null
      at += key.length
    }
    while (line[at] === ' ' || line[at] === '\t') at++
    if (line[at] !== '=') {
      name = key
      continue
    }
    at++
    while (line[at] === ' ' || line[at] === '\t') at++
    const value = readQuoted(line, at)
    if (!value) return null
    labels[key] = value.text
    at = value.end
  }
}

function readQuoted(
  line: string,
  start: number,
): { text: string; end: number } | null {
  if (line[start] !== '"') return null
  let at = start + 1
  let raw = ''
  while (at < line.length) {
    const char = line[at]
    if (char === '\\') {
      raw += line.slice(at, at + 2)
      at += 2
      continue
    }
    if (char === '"') return { text: unescapeText(raw, true), end: at + 1 }
    raw += char
    at++
  }
  return null
}

/** `\\` and `\n` (and in a label value `\"`); any other backslash stays as written. */
function unescapeText(text: string, quoted: boolean): string {
  return text.replace(/\\(.)/g, (whole, char: string) => {
    if (char === 'n') return '\n'
    if (char === '\\') return '\\'
    if (char === '"' && quoted) return '"'
    return whole
  })
}

/** Go's `ParseFloat` as Prometheus uses it: decimals, exponents, `NaN`, `±Inf`; undefined if not. */
function parseValue(text: string): number | undefined {
  const lower = text.toLowerCase()
  if (lower === 'nan') return Number.NaN
  if (['+inf', 'inf', '+infinity', 'infinity'].includes(lower))
    return Number.POSITIVE_INFINITY
  if (lower === '-inf' || lower === '-infinity') return Number.NEGATIVE_INFINITY
  if (text === '' || !/^[+-]?(\d|\.\d)/.test(text)) return undefined
  const value = Number(text)
  return Number.isNaN(value) ? undefined : value
}

/** Each sample to its family: a declared name, or a declared name plus its type's suffix. */
function familyOf(
  name: string,
  declared: Map<string, Declared>,
): { family: string; suffix: string } {
  if (declared.has(name)) return { family: name, suffix: '' }
  for (const [type, suffixes] of Object.entries(SUFFIXES)) {
    for (const suffix of suffixes) {
      if (!name.endsWith(suffix)) continue
      const base = name.slice(0, -suffix.length)
      if (declared.get(base)?.type === type) return { family: base, suffix }
    }
  }
  return { family: name, suffix: '' }
}

function build(
  declared: Map<string, Declared>,
  samples: Sample[],
): MetricFamily[] {
  const grouped = new Map<string, Array<Sample & { suffix: string }>>()
  for (const sample of samples) {
    const { family, suffix } = familyOf(sample.name, declared)
    const list = grouped.get(family) ?? []
    list.push({ ...sample, suffix })
    grouped.set(family, list)
  }
  const families: MetricFamily[] = []
  for (const [name, members] of grouped) {
    const meta = declared.get(name) ?? { type: 'untyped', help: null }
    if (meta.type === 'histogram' || meta.type === 'gaugehistogram')
      families.push(histogram(name, meta, members))
    else if (meta.type === 'counter')
      families.push({
        // 0.0.4 declares `neo_runs_total`; OpenMetrics declares `neo_runs` and samples `_total`.
        name: name.endsWith('_total') ? name : `${name}_total`,
        kind: 'counter',
        help: meta.help,
        series: members
          .filter(
            (member) => member.suffix === '' || member.suffix === '_total',
          )
          .map(({ labels, value }) => ({ labels, value: finite(value) })),
      })
    else
      families.push({
        name,
        kind: meta.type === 'gauge' ? 'gauge' : 'unknown',
        help: meta.help,
        // A summary keeps its quantiles; its `_sum`/`_count` and an info's `_info` stay out.
        series: members
          .filter((member) => member.suffix === '' || member.suffix === '_info')
          .map(({ labels, value }) => ({ labels, value: finite(value) })),
      })
  }
  return families
}

/** `_bucket{le}`, `_sum` and `_count` samples grouped into one series per label set (without `le`). */
function histogram(
  name: string,
  meta: Declared,
  members: Array<Sample & { suffix: string }>,
): MetricFamily {
  const bySet = new Map<
    string,
    {
      labels: Record<string, string>
      count: number | null
      sum: number | null
      buckets: Array<{ le: number | null; count: number }>
    }
  >()
  for (const member of members) {
    const { le, ...labels } = member.labels
    const key = JSON.stringify(Object.entries(labels).sort())
    const series = bySet.get(key) ?? {
      labels,
      count: null,
      sum: null,
      buckets: [],
    }
    bySet.set(key, series)
    if (member.suffix === '_bucket' && le !== undefined) {
      const bound = parseValue(le)
      if (bound === undefined || Number.isNaN(bound)) continue
      series.buckets.push({
        le: bound === Number.POSITIVE_INFINITY ? null : bound,
        count: member.value,
      })
    } else if (member.suffix === '_count' || member.suffix === '_gcount')
      series.count = member.value
    else if (member.suffix === '_sum' || member.suffix === '_gsum')
      series.sum = finite(member.value)
  }
  return {
    name,
    kind: 'histogram',
    help: meta.help,
    series: [...bySet.values()].map((series) => {
      const buckets = series.buckets.sort(
        (a, b) =>
          (a.le ?? Number.POSITIVE_INFINITY) -
          (b.le ?? Number.POSITIVE_INFINITY),
      )
      const inf = buckets.find((bucket) => bucket.le === null)?.count
      return {
        labels: series.labels,
        value: finite(series.count ?? inf ?? 0),
        sum: series.sum,
        buckets,
      }
    }),
  }
}

function finite(value: number): number | null {
  return Number.isFinite(value) ? value : null
}
