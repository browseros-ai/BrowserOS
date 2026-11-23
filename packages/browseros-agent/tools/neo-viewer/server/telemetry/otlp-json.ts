/**
 * OTLP/HTTP JSON, decoded into the viewer's own types: Neo's two spans (`NeoSpan`) and metric
 * families (`MetricFamily`, the model the Prometheus parser produces too). Pure: no I/O, no state.
 *
 * The JSON encoding is OTLP's (opentelemetry-proto, "JSON Protobuf Encoding"): trace and span ids
 * are hex strings, 64-bit integers are strings (numbers accepted too), enums are integers (their
 * names accepted too), doubles may be the strings `NaN`, `Infinity`, `-Infinity`. Nothing here
 * depends on a protobuf library; the viewer refuses protobuf bodies before they get here.
 *
 * Neo sends two kinds of span (wire-contract.md "Spans"): `neo.run`, and one per operation named
 * after it (`page.goto`, `unknown`). A span says which it is in `neo.span.type` (`run` or `op`),
 * never by its name, which for an operation is any method Neo serves. Anything else, or a Neo span
 * missing the id that names its run, is counted as rejected and dropped: the viewer files every
 * span under `neo.run.id`, and a span it cannot file has nowhere to go.
 */

import type { MetricFamily } from '../../shared/api'
import { asScraped } from './otlp-names'

/** A span attribute as the viewer keeps it: scalars as sent; arrays and maps as JSON text. */
export type Attribute = string | number | boolean
export type Attributes = Record<string, Attribute>

export interface SpanError {
  owner: string
  code: string
  /** The span's status message; empty when Neo set none. */
  message: string
}

interface SpanBase {
  traceId: string
  spanId: string
  parentSpanId: string | null
  /** Epoch ms, microsecond precision. */
  startMs: number
  endMs: number
  /** The OTel span name: `neo.run`, or an operation's name as its metric label has it. */
  spanName: string
  /** OTel status `ERROR`. */
  failed: boolean
  attributes: Attributes
  /** The sender's resource (`service.name`, …), shared by every span of its batch. */
  resource: Attributes
}

/** `neo.run`: one per admitted run. */
export interface RunSpan extends SpanBase {
  kind: 'run'
  /** `neo.run.id`, the id `GET /v1/runs/{id}` and the reply's `_meta` use. */
  neoRunId: string
  /** `neo.run.status`: `ok`, `error`, `cancelled`, `timeout`, … */
  status: string
  taskId: string | null
  error: SpanError | null
}

/** One per dispatched operation (`neo.span.type` `op`), a child of its run's `neo.run`. */
export interface OpSpan extends SpanBase {
  kind: 'op'
  /** `neo.op.id`: `<neoRunId>.<seq>`, as `/v1/runs/{id}` numbers operations. */
  opId: string
  neoRunId: string
  seq: number
  /** The raw name the script called (`neo.op.name`). */
  name: string
  site: { line: number; column: number } | null
  tabId: number | null
  /** `neo.op.outcome`: `ok`, `error`, `cancelled`. */
  outcome: string
  /** `neo.op.delivery`: `done`, `not_sent`, `unknown`; null when absent. */
  delivery: string | null
  error: SpanError | null
}

export type NeoSpan = RunSpan | OpSpan

export interface Decoded<T> {
  items: T[]
  /** Entries dropped as invalid (OTLP's `partialSuccess.rejected*`). */
  rejected: number
}

/** The body is not OTLP at all (a 400, not a partial success). */
export class NotOtlp extends Error {}

/**
 * Attributes kept per span, the longest key kept (a longer one drops its attribute) and the
 * longest string value or status message kept (a longer one ends in `…`): what one span may hold
 * is bounded, so the span store's byte cap means something.
 */
const MAX_ATTRIBUTES = 64
const MAX_KEY_CHARS = 256
const MAX_VALUE_CHARS = 4_096
const HEX_TRACE = /^[0-9a-f]{32}$/
const HEX_SPAN = /^[0-9a-f]{16}$/

type Json = Record<string, unknown>

/** `ExportTraceServiceRequest` → Neo's spans. */
export function decodeSpans(body: unknown): Decoded<NeoSpan> {
  const resourceSpans = listField(body, 'resourceSpans')
  const items: NeoSpan[] = []
  let rejected = 0
  for (const block of resourceSpans) {
    const resource = attributesOf(field(field(block, 'resource'), 'attributes'))
    for (const scope of [
      ...listOf(field(block, 'scopeSpans')),
      // OTLP before 0.18 named the scope "instrumentation library".
      ...listOf(field(block, 'instrumentationLibrarySpans')),
    ]) {
      for (const raw of listOf(field(scope, 'spans'))) {
        const span = decodeSpan(raw, resource)
        if (span) items.push(span)
        else rejected++
      }
    }
  }
  return { items, rejected }
}

function decodeSpan(raw: unknown, resource: Attributes): NeoSpan | null {
  if (!isObject(raw)) return null
  const traceId = hexId(raw.traceId, HEX_TRACE)
  const spanId = hexId(raw.spanId, HEX_SPAN)
  if (!traceId || !spanId) return null
  // A root span has no parent: an empty id, or (some exporters) an all-zero one.
  const parent = raw.parentSpanId
  const parentSpanId =
    parent === undefined ||
    parent === null ||
    parent === '' ||
    parent === '0000000000000000'
      ? null
      : hexId(parent, HEX_SPAN)
  if (parentSpanId === undefined) return null
  const startMs = nanosToMs(raw.startTimeUnixNano)
  const endMs = Math.max(startMs, nanosToMs(raw.endTimeUnixNano))
  const status = isObject(raw.status) ? raw.status : {}
  const failed = status.code === 2 || status.code === 'STATUS_CODE_ERROR'
  const message = typeof status.message === 'string' ? clip(status.message) : ''
  const attributes = attributesOf(raw.attributes)
  const base: SpanBase = {
    traceId,
    spanId,
    parentSpanId,
    startMs,
    endMs,
    spanName: typeof raw.name === 'string' ? clip(raw.name) : '',
    failed,
    attributes,
    resource,
  }
  const error = spanError(attributes, message)
  const type = attributes['neo.span.type']
  if (type === 'run') return runSpan(base, error)
  if (type === 'op') return opSpan(base, error)
  return null
}

/** A span typed `run`, or null without the run id that files it. */
function runSpan(base: SpanBase, error: SpanError | null): RunSpan | null {
  const { attributes, failed } = base
  const neoRunId = text(attributes['neo.run.id'])
  if (!neoRunId) return null
  return {
    ...base,
    kind: 'run',
    neoRunId,
    status: text(attributes['neo.run.status']) ?? (failed ? 'error' : 'ok'),
    taskId: text(attributes['neo.task.id']),
    error,
  }
}

/** A span typed `op`, or null without an op id (`<neoRunId>.<seq>`) that files it. */
function opSpan(base: SpanBase, error: SpanError | null): OpSpan | null {
  const { attributes, failed } = base
  const opId = text(attributes['neo.op.id'])
  const dot = opId ? opId.lastIndexOf('.') : -1
  const seq = opId ? Number(opId.slice(dot + 1)) : Number.NaN
  if (!opId || dot <= 0 || !Number.isInteger(seq) || seq < 1) return null
  const line = attributes['code.line.number']
  const column = attributes['code.column.number']
  return {
    ...base,
    kind: 'op',
    opId,
    neoRunId: opId.slice(0, dot),
    seq,
    name: text(attributes['neo.op.name']) ?? '',
    site:
      typeof line === 'number' && typeof column === 'number'
        ? { line, column }
        : null,
    tabId: tabIdOf(attributes['neo.tab.id']),
    outcome: text(attributes['neo.op.outcome']) ?? (failed ? 'error' : 'ok'),
    delivery: text(attributes['neo.op.delivery']),
    error,
  }
}

/** `neo.error.owner` and `neo.error.code`, set together on a failed run or operation. */
function spanError(attributes: Attributes, message: string): SpanError | null {
  const owner = text(attributes['neo.error.owner'])
  const code = text(attributes['neo.error.code'])
  return owner || code
    ? { owner: owner ?? '', code: code ?? '', message }
    : null
}

/** Neo's API reports a tab id as a number; a numeric string is read as one. */
function tabIdOf(value: Attribute | undefined): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value
  if (typeof value === 'string' && /^\d+$/.test(value)) return Number(value)
  return null
}

/**
 * `ExportMetricsServiceRequest` → metric families, one per name. Metrics arrive under
 * OpenTelemetry's names and are filed under the Prometheus names a scrape derives from them
 * (`otlp-names.ts`), so both doors give one model.
 */
export function decodeMetrics(body: unknown): Decoded<MetricFamily> {
  const byName = new Map<string, MetricFamily>()
  let rejected = 0
  for (const block of listField(body, 'resourceMetrics')) {
    for (const scope of [
      ...listOf(field(block, 'scopeMetrics')),
      ...listOf(field(block, 'instrumentationLibraryMetrics')),
    ]) {
      for (const raw of listOf(field(scope, 'metrics'))) {
        const decoded = decodeMetric(raw)
        rejected += decoded.rejected
        if (!decoded.family) continue
        const unit = field(raw, 'unit')
        const family = asScraped(
          decoded.family,
          typeof unit === 'string' ? unit : '',
        )
        const known = byName.get(family.name)
        if (known && known.kind === family.kind)
          known.series.push(...family.series)
        else if (!known) byName.set(family.name, family)
        else rejected += family.series.length
      }
    }
  }
  return { items: [...byName.values()], rejected }
}

function decodeMetric(raw: unknown): {
  family: MetricFamily | null
  rejected: number
} {
  if (!isObject(raw) || typeof raw.name !== 'string' || raw.name === '')
    return { family: null, rejected: 1 }
  const family: MetricFamily = {
    name: raw.name,
    kind: 'unknown',
    help:
      typeof raw.description === 'string' && raw.description !== ''
        ? raw.description
        : null,
    series: [],
  }
  if (isObject(raw.sum) || isObject(raw.gauge)) {
    // A monotonic sum is a counter; an up-down counter (`neo.run.active`, `neo.tab.count`) is
    // a gauge, as Prometheus renders it.
    family.kind =
      isObject(raw.sum) && raw.sum.isMonotonic === true ? 'counter' : 'gauge'
    const data = (isObject(raw.sum) ? raw.sum : raw.gauge) as Json
    return fill(family, data.dataPoints, (point) => {
      const value = pointValue(point)
      return value === undefined
        ? null
        : { labels: labelsOf(point.attributes), value }
    })
  }
  if (isObject(raw.histogram)) {
    family.kind = 'histogram'
    return fill(family, raw.histogram.dataPoints, histogramPoint)
  }
  // Exponential histograms and summaries: Neo sends neither; keep their counts, so the raw view
  // still lists them.
  const other = isObject(raw.exponentialHistogram)
    ? raw.exponentialHistogram
    : isObject(raw.summary)
      ? raw.summary
      : null
  if (!other) return { family: null, rejected: 1 }
  return fill(family, other.dataPoints, (point) => ({
    labels: labelsOf(point.attributes),
    value: finiteOrNull(int64(point.count)),
    sum: finiteOrNull(double(point.sum)),
  }))
}

/** The family with a series per readable data point; the rest are counted as rejected. */
function fill(
  family: MetricFamily,
  points: unknown,
  read: (point: Json) => MetricFamily['series'][number] | null,
): { family: MetricFamily; rejected: number } {
  let rejected = 0
  for (const point of listOf(points)) {
    const series = isObject(point) ? read(point) : null
    if (series) family.series.push(series)
    else rejected++
  }
  return { family, rejected }
}

/** `asDouble` or `asInt`; undefined when the point has neither (null: NaN or ±Inf). */
function pointValue(point: Json): number | null | undefined {
  if ('asDouble' in point) {
    const value = double(point.asDouble)
    return value === undefined ? undefined : finiteOrNull(value)
  }
  if ('asInt' in point) {
    const value = int64(point.asInt)
    return value === undefined ? undefined : finiteOrNull(value)
  }
  return undefined
}

/**
 * OTLP buckets count observations per bucket (`bucketCounts[i]` ≤ `explicitBounds[i]`, the last
 * above every bound); the viewer's model is Prometheus' cumulative `le` buckets ending in +Inf.
 */
function histogramPoint(point: Json): MetricFamily['series'][number] | null {
  const count = int64(point.count)
  if (count === undefined) return null
  const bounds = listOf(point.explicitBounds).map(double)
  const counts = listOf(point.bucketCounts).map(int64)
  if (
    bounds.some((bound) => bound === undefined) ||
    counts.some((n) => n === undefined) ||
    (counts.length > 0 && counts.length !== bounds.length + 1)
  )
    return null
  let running = 0
  const buckets = counts.map((n, index) => {
    running += n as number
    return {
      le: index < bounds.length ? (bounds[index] as number) : null,
      count: running,
    }
  })
  return {
    labels: labelsOf(point.attributes),
    value: count,
    sum: 'sum' in point ? finiteOrNull(double(point.sum)) : null,
    buckets,
  }
}

// ─── OTLP JSON values ────────────────────────────────────────────────────────────────────────

/** `KeyValue[]` → attributes, bounded. */
function attributesOf(list: unknown): Attributes {
  const out: Attributes = {}
  let kept = 0
  for (const entry of listOf(list)) {
    if (kept >= MAX_ATTRIBUTES) break
    if (
      !isObject(entry) ||
      typeof entry.key !== 'string' ||
      entry.key.length > MAX_KEY_CHARS
    )
      continue
    const value = anyValue(entry.value)
    if (value === undefined) continue
    out[entry.key] = typeof value === 'string' ? clip(value) : value
    kept++
  }
  return out
}

/** Data point attributes as metric labels: every value as text. */
function labelsOf(list: unknown): Record<string, string> {
  const labels: Record<string, string> = {}
  for (const [key, value] of Object.entries(attributesOf(list)))
    labels[key] = String(value)
  return labels
}

/** An `AnyValue`; arrays and key-value lists become their JSON text. */
function anyValue(value: unknown): Attribute | undefined {
  if (!isObject(value)) return undefined
  if (typeof value.stringValue === 'string') return value.stringValue
  if (typeof value.boolValue === 'boolean') return value.boolValue
  if ('intValue' in value) {
    const n = int64(value.intValue)
    // Past 2^53 a number would lose digits: keep the text.
    if (n !== undefined && Number.isSafeInteger(n)) return n
    return typeof value.intValue === 'string' ? value.intValue : undefined
  }
  if ('doubleValue' in value) {
    const n = double(value.doubleValue)
    return n === undefined ? undefined : Number.isFinite(n) ? n : String(n)
  }
  if (typeof value.bytesValue === 'string') return value.bytesValue
  if (isObject(value.arrayValue))
    return JSON.stringify(listOf(value.arrayValue.values).map(anyValue))
  if (isObject(value.kvlistValue))
    return JSON.stringify(attributesOf(value.kvlistValue.values))
  return undefined
}

/** A uint64/int64 as JSON gives it: a decimal string, or a number. */
function int64(value: unknown): number | undefined {
  if (typeof value === 'number')
    return Number.isInteger(value) ? value : undefined
  if (typeof value === 'string' && /^-?\d+$/.test(value)) return Number(value)
  return undefined
}

/** A double as proto3 JSON gives it: a number, or `NaN`, `Infinity`, `-Infinity` as text. */
function double(value: unknown): number | undefined {
  if (typeof value === 'number') return value
  if (value === 'NaN') return Number.NaN
  if (value === 'Infinity') return Number.POSITIVE_INFINITY
  if (value === '-Infinity') return Number.NEGATIVE_INFINITY
  if (
    typeof value === 'string' &&
    value.trim() !== '' &&
    !Number.isNaN(Number(value))
  )
    return Number(value)
  return undefined
}

/**
 * Unix nanoseconds (a decimal string, as OTLP JSON sends a fixed64) → epoch ms with microsecond
 * precision. Nanoseconds since 1970 pass 2^53, so a string is cut in BigInt before it is a number.
 */
function nanosToMs(value: unknown): number {
  if (typeof value === 'string' && /^\d+$/.test(value))
    return Number(BigInt(value) / 1_000n) / 1_000
  if (typeof value === 'number' && Number.isFinite(value)) return value / 1e6
  return 0
}

/** OTLP JSON ids are hex (not base64, as plain proto3 JSON would have them); all-zero is invalid. */
function hexId(value: unknown, shape: RegExp): string | undefined {
  if (typeof value !== 'string') return undefined
  const hex = value.toLowerCase()
  return shape.test(hex) && /[^0]/.test(hex) ? hex : undefined
}

function finiteOrNull(value: number | undefined): number | null {
  return value !== undefined && Number.isFinite(value) ? value : null
}

function text(value: Attribute | undefined): string | null {
  return typeof value === 'string' && value !== '' ? value : null
}

function clip(value: string): string {
  return value.length > MAX_VALUE_CHARS
    ? `${value.slice(0, MAX_VALUE_CHARS)}…`
    : value
}

/** The top-level list a request must hold; absent is an empty request, anything else is not OTLP. */
function listField(body: unknown, name: string): unknown[] {
  if (!isObject(body)) throw new NotOtlp('the body is not a JSON object')
  const value = body[name]
  if (value === undefined || value === null) return []
  if (!Array.isArray(value)) throw new NotOtlp(`"${name}" is not an array`)
  return value
}

function field(value: unknown, name: string): unknown {
  return isObject(value) ? value[name] : undefined
}

function listOf(value: unknown): unknown[] {
  return Array.isArray(value) ? value : []
}

function isObject(value: unknown): value is Json {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}
