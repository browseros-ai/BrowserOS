/**
 * A pushed metric under the name a scrape gives it. Neo's OTLP push sends OpenTelemetry names
 * and attribute keys (`neo.run.duration` with `neo.run.status`); `GET /v1/metrics` serves the
 * Prometheus names OpenTelemetry's compatibility rules derive from them
 * (`neo_run_duration_seconds` with `neo_run_status`), the same translation a collector's
 * Prometheus exporter or Prometheus' own OTLP receiver applies. The viewer applies it too, so the
 * Health view and health bar read one set of names whichever door answered (openapi.yaml,
 * `/v1/metrics`).
 */

import type { MetricFamily } from '../../shared/api'

/** The UCUM units Prometheus spells out (the compatibility spec's table). */
const UNIT_WORDS: Readonly<Record<string, string>> = {
  d: 'days',
  h: 'hours',
  min: 'minutes',
  s: 'seconds',
  ms: 'milliseconds',
  us: 'microseconds',
  ns: 'nanoseconds',
  By: 'bytes',
  KiBy: 'kibibytes',
  MiBy: 'mebibytes',
  GiBy: 'gibibytes',
  TiBy: 'tebibytes',
  kBy: 'kilobytes',
  MBy: 'megabytes',
  GBy: 'gigabytes',
  TBy: 'terabytes',
  m: 'meters',
  V: 'volts',
  A: 'amperes',
  J: 'joules',
  W: 'watts',
  g: 'grams',
  Cel: 'celsius',
  Hz: 'hertz',
  '%': 'percent',
}

/** A rate's time unit in the singular, as Prometheus spells it after `per_`. */
const PER_UNIT_WORDS: Readonly<Record<string, string>> = {
  s: 'second',
  m: 'minute',
  h: 'hour',
  d: 'day',
  w: 'week',
  mo: 'month',
  y: 'year',
}

/** A pushed family as a scrape names it: its name and every series' label names. */
export function asScraped(family: MetricFamily, unit: string): MetricFamily {
  return {
    ...family,
    name: familyName(family.name, family.kind, unit),
    series: family.series.map((series) => ({
      ...series,
      labels: Object.fromEntries(
        Object.entries(series.labels).map(([key, value]) => [
          labelName(key),
          value,
        ]),
      ),
    })),
  }
}

/**
 * The Prometheus name of an OpenTelemetry metric: the unit's word follows the name unless it
 * already ends with it (`s` is `seconds`; an annotation such as `{run}` adds nothing), a counter
 * ends in `_total`, every character outside `[a-zA-Z0-9_:]` is `_`, and runs of `_` are one.
 */
export function familyName(
  name: string,
  kind: MetricFamily['kind'],
  unit: string,
): string {
  let scraped = name
  const word = unitWord(unit)
  if (word && !scraped.endsWith(`_${word}`)) scraped += `_${word}`
  if (kind === 'counter' && !scraped.endsWith('_total')) scraped += '_total'
  return sanitized(scraped, /[^a-zA-Z0-9_:]/g)
}

/** An attribute key as a label name: `neo.op.name` is `neo_op_name`. */
export function labelName(key: string): string {
  if (key === '') return '_'
  const name = sanitized(key, /[^a-zA-Z0-9_]/g)
  return /^[0-9]/.test(name) ? `key_${name}` : name
}

function unitWord(unit: string): string | null {
  const bare = unit.replace(/\{[^}]*\}/g, '')
  const [main = '', per = ''] = bare.split('/', 2)
  const mainWord = main ? (lookup(UNIT_WORDS, main) ?? main) : ''
  const perWord = per
    ? (lookup(PER_UNIT_WORDS, per) ?? lookup(UNIT_WORDS, per) ?? per)
    : ''
  if (mainWord && perWord) return `${mainWord}_per_${perWord}`
  if (perWord) return `per_${perWord}`
  return mainWord || null
}

function sanitized(text: string, disallowed: RegExp): string {
  return text.replace(disallowed, '_').replace(/_+/g, '_')
}

function lookup(
  table: Readonly<Record<string, string>>,
  key: string,
): string | undefined {
  return Object.hasOwn(table, key) ? table[key] : undefined
}
