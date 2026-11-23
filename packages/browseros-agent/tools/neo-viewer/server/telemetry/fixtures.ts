/**
 * Neo's telemetry for one realistic run, as Neo emits it (wire-contract.md): a `neo.run` span and
 * eight operation spans, each named after its op (one nested in another, one failed), the 11
 * metrics as Prometheus text and as OTLP JSON, and the matching `GET /v1/runs/{id}` record. The tests and the feeder
 * (`feed-fixture.ts`) both use it, so what the page is built against is what the tests check.
 *
 * Span ids derive from the run id and the op's seq, so sending the same run twice is an
 * exporter's retry (the viewer keeps one copy), not a second run.
 */

import { createHash } from 'node:crypto'
import type { MetricFamily } from '../../shared/api'

/** The script the fixture's spans describe, one statement per line. */
export const FIXTURE_CODE = [
  "await page.goto('https://example.com')",
  "const heading = await page.locator('h1').textContent()",
  "const links = await page.locator('a').count()",
  "await page.getByRole('link', { name: 'More information' }).click()",
  "await page.waitForLoadState('domcontentloaded')",
  "try { await page.locator('#missing').click({ timeout: 500 }) } catch {}",
  'const title = await page.title()',
  'return { heading, links, title }',
].join('\n')

interface FixtureOp {
  seq: number
  name: string
  /** Ms from the run's start. */
  start: number
  end: number
  line: number | null
  /** The op this one ran inside. */
  parent?: number
  error?: { owner: string; code: string; message: string }
}

/** Eight operations; seq 5 runs inside seq 4 (the click waits for the navigation it caused). */
export const FIXTURE_OPS: FixtureOp[] = [
  { seq: 1, name: 'page.goto', start: 5, end: 420, line: 1 },
  { seq: 2, name: 'locator.textContent', start: 425, end: 450, line: 2 },
  { seq: 3, name: 'locator.count', start: 452, end: 470, line: 3 },
  { seq: 4, name: 'locator.click', start: 472, end: 640, line: 4 },
  {
    seq: 5,
    name: 'frame.waitForLoadState',
    start: 500,
    end: 620,
    line: null,
    parent: 4,
  },
  { seq: 6, name: 'page.waitForLoadState', start: 642, end: 700, line: 5 },
  {
    seq: 7,
    name: 'locator.click',
    start: 702,
    end: 1210,
    line: 6,
    error: {
      owner: 'script',
      code: 'no_match',
      message: 'locator.click: Timeout 500ms exceeded.',
    },
  },
  { seq: 8, name: 'page.title', start: 1212, end: 1220, line: 7 },
]
export const FIXTURE_RUN_MS = 1230
const TAB_ID = 7

export interface FixtureRun {
  /** Neo's run id: the reply's `_meta` id and `neo.run.id`. */
  neoRunId: string
  traceId: string
  /** The span `neo.run` hangs under (the caller's, or the one the viewer minted); null: a root. */
  parentSpanId: string | null
  /** Epoch ms the run started. */
  startMs: number
}

/** A stable 16-hex span id for one span of one run. */
export function fixtureSpanId(neoRunId: string, seq: number): string {
  return createHash('sha256')
    .update(`${neoRunId}/${seq}`)
    .digest('hex')
    .slice(0, 16)
}

/** The run's `ExportTraceServiceRequest`, as `OTEL_EXPORTER_OTLP_PROTOCOL=http/json` sends it. */
export function fixtureTraces(
  run: FixtureRun,
  options: { ops?: number[] } = {},
) {
  const runSpanId = fixtureSpanId(run.neoRunId, 0)
  const wanted = options.ops ? new Set(options.ops) : null
  const ops = FIXTURE_OPS.filter((op) => !wanted || wanted.has(op.seq))
  const spans = ops.map((op) => ({
    traceId: run.traceId,
    spanId: fixtureSpanId(run.neoRunId, op.seq),
    parentSpanId: op.parent
      ? fixtureSpanId(run.neoRunId, op.parent)
      : runSpanId,
    name: op.name,
    kind: 1,
    startTimeUnixNano: nanos(run.startMs + op.start),
    endTimeUnixNano: nanos(run.startMs + op.end),
    attributes: [
      string('neo.span.type', 'op'),
      string('neo.op.name', op.name),
      string('neo.op.id', `${run.neoRunId}.${op.seq}`),
      ...(op.line === null
        ? []
        : [int('neo.op.site.line', op.line), int('neo.op.site.column', 7)]),
      int('neo.tab.id', TAB_ID),
      string('neo.op.outcome', op.error ? 'error' : 'ok'),
      string('neo.op.delivery', op.error ? 'not_sent' : 'done'),
      ...(op.error
        ? [
            string('neo.error.owner', op.error.owner),
            string('neo.error.code', op.error.code),
          ]
        : []),
    ],
    status: op.error ? { code: 2, message: op.error.message } : {},
  }))
  const runSpan =
    options.ops && !options.ops.includes(0)
      ? []
      : [
          {
            traceId: run.traceId,
            spanId: runSpanId,
            parentSpanId: run.parentSpanId ?? '',
            name: 'neo.run',
            kind: 2,
            startTimeUnixNano: nanos(run.startMs),
            endTimeUnixNano: nanos(run.startMs + FIXTURE_RUN_MS),
            attributes: [
              string('neo.span.type', 'run'),
              string('neo.run.id', run.neoRunId),
              string('neo.run.status', 'ok'),
            ],
            status: {},
          },
        ]
  return {
    resourceSpans: [
      {
        resource: { attributes: RESOURCE },
        scopeSpans: [
          {
            scope: { name: 'neo-telemetry' },
            spans: [...spans, ...runSpan],
          },
        ],
      },
    ],
  }
}

/** What Neo's host API says about the same run (`GET /v1/runs/{id}`, openapi.yaml `Run`). */
export function fixtureHostRun(run: FixtureRun) {
  return {
    id: run.neoRunId,
    status: 'ok',
    effects: 'done',
    startedAt: Math.round(run.startMs),
    endedAt: Math.round(run.startMs + FIXTURE_RUN_MS),
    complete: true,
    tabs: [TAB_ID],
    error: null,
    operations: FIXTURE_OPS.map((op) => ({
      id: `${run.neoRunId}.${op.seq}`,
      ...(op.parent ? { parentId: `${run.neoRunId}.${op.parent}` } : {}),
      name: op.name,
      kind: 'builtin',
      effect:
        op.name.endsWith('click') || op.name === 'page.goto' ? 'write' : 'read',
      ...(op.line === null ? {} : { site: { line: op.line, column: 7 } }),
      tabId: TAB_ID,
      args: op.name === 'page.goto' ? { url: 'https://example.com' } : null,
      result: null,
      startedAt: Math.round(run.startMs + op.start),
      durationMs: op.end - op.start,
      outcome: op.error ? 'error' : 'ok',
      delivery: op.error ? 'not_sent' : 'done',
      ...(op.error ? { error: { ...op.error, retryable: true } } : {}),
    })),
  }
}

/** The bucket bounds Neo's histograms use in the fixture (consumers read bounds from the data). */
const BOUNDS = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10]

/**
 * All 11 metrics, cumulative. `step` moves them on as time does (each step one more run and its
 * operations), so a feeder pushing every 500 ms shows the page live numbers.
 */
export function fixtureFamilies(step = 0): MetricFamily[] {
  const runs = 3 + step
  const counter = (
    name: string,
    help: string,
    series: Array<[Record<string, string>, number]>,
  ): MetricFamily => ({
    name,
    kind: 'counter',
    help,
    series: series.map(([labels, value]) => ({ labels, value })),
  })
  const gauge = (
    name: string,
    help: string,
    series: Array<[Record<string, string>, number]>,
  ): MetricFamily => ({ ...counter(name, help, series), kind: 'gauge' })
  return [
    counter('neo_runs_total', 'Runs ended, by status and fault owner.', [
      [{ status: 'ok', error_owner: 'none' }, runs],
      [{ status: 'error', error_owner: 'script' }, 1],
    ]),
    gauge('neo_runs_in_flight', 'Runs running now.', [[{}, step % 2]]),
    histogram('neo_run_duration_seconds', 'Run duration.', [
      [{ status: 'ok' }, Array(runs).fill(1.23)],
      [{ status: 'error' }, [0.61]],
    ]),
    counter(
      'neo_operations_total',
      'Operations settled, by op, status and fault owner.',
      [
        [{ op: 'page.goto', status: 'ok', error_owner: 'none' }, runs],
        [
          { op: 'locator.textContent', status: 'ok', error_owner: 'none' },
          runs,
        ],
        [{ op: 'locator.count', status: 'ok', error_owner: 'none' }, runs],
        [{ op: 'locator.click', status: 'ok', error_owner: 'none' }, runs],
        [
          { op: 'locator.click', status: 'error', error_owner: 'script' },
          runs + 1,
        ],
        [
          { op: 'frame.waitForLoadState', status: 'ok', error_owner: 'none' },
          runs,
        ],
        [
          { op: 'page.waitForLoadState', status: 'ok', error_owner: 'none' },
          runs,
        ],
        [{ op: 'page.title', status: 'ok', error_owner: 'none' }, runs],
      ],
    ),
    histogram('neo_operation_duration_seconds', 'Operation duration.', [
      [{ op: 'page.goto', status: 'ok' }, Array(runs).fill(0.415)],
      [{ op: 'locator.click', status: 'ok' }, Array(runs).fill(0.168)],
      [{ op: 'locator.click', status: 'error' }, Array(runs + 1).fill(0.508)],
      [{ op: 'page.title', status: 'ok' }, Array(runs).fill(0.008)],
    ]),
    counter('neo_errors_total', 'Errors, each counted once where attributed.', [
      [{ error_owner: 'script', error_code: 'no_match' }, runs + 1],
      [{ error_owner: 'website', error_code: 'navigation_failed' }, 1],
    ]),
    gauge('neo_tabs_open', 'Tabs open now, by origin.', [
      [{ origin: 'agent' }, 2],
      [{ origin: 'found' }, 1],
    ]),
    counter('neo_tabs_opened_total', 'Tabs opened, by origin.', [
      [{ origin: 'agent' }, 3 + step],
      [{ origin: 'found' }, 1],
    ]),
    counter('neo_tabs_closed_total', 'Tabs closed, by origin and reason.', [
      [{ origin: 'agent', close_reason: 'script' }, 1 + step],
      [{ origin: 'agent', close_reason: 'janitor' }, 0],
    ]),
    histogram('neo_tab_lifetime_seconds', 'Tab lifetime before close.', [
      [{ origin: 'agent', close_reason: 'script' }, Array(1 + step).fill(42)],
    ]),
    counter('neo_renderer_crashes_total', 'Renderer crashes.', [[{}, 0]]),
  ]
}

/** The families as `GET /v1/metrics` serves them. */
export function fixturePrometheus(step = 0): string {
  return renderPrometheus(fixtureFamilies(step))
}

/** Each family's OTLP unit, as Neo's catalog sets it. */
const UNITS: Record<string, string> = {
  neo_runs_total: '{run}',
  neo_runs_in_flight: '{run}',
  neo_run_duration_seconds: 's',
  neo_operations_total: '{operation}',
  neo_operation_duration_seconds: 's',
  neo_errors_total: '{error}',
  neo_tabs_open: '{tab}',
  neo_tabs_opened_total: '{tab}',
  neo_tabs_closed_total: '{tab}',
  neo_tab_lifetime_seconds: 's',
  neo_renderer_crashes_total: '{crash}',
}

/** The families as an `ExportMetricsServiceRequest` (cumulative temporality). */
export function fixtureMetrics(step = 0, at = Date.now()) {
  const time = nanos(at)
  const start = nanos(at - 60_000)
  const metrics = fixtureFamilies(step).map((family) => {
    const unit = UNITS[family.name] ?? ''
    const head = { name: family.name, description: family.help ?? '', unit }
    if (family.kind === 'histogram')
      return {
        ...head,
        histogram: {
          aggregationTemporality: 2,
          dataPoints: family.series.map((series) => {
            const buckets = series.buckets ?? []
            return {
              attributes: attributes(series.labels),
              startTimeUnixNano: start,
              timeUnixNano: time,
              count: String(series.value ?? 0),
              sum: series.sum ?? 0,
              explicitBounds: buckets.flatMap((bucket) =>
                bucket.le === null ? [] : [bucket.le],
              ),
              // OTLP counts per bucket, not cumulatively.
              bucketCounts: buckets.map((bucket, index) =>
                String(bucket.count - (buckets[index - 1]?.count ?? 0)),
              ),
            }
          }),
        },
      }
    const points = family.series.map((series) => ({
      attributes: attributes(series.labels),
      startTimeUnixNano: start,
      timeUnixNano: time,
      asInt: String(series.value ?? 0),
    }))
    if (family.kind === 'counter')
      return {
        ...head,
        sum: {
          aggregationTemporality: 2,
          isMonotonic: true,
          dataPoints: points,
        },
      }
    // Neo's two gauges (`neo_runs_in_flight`, `neo_tabs_open`) are up-down counters, as their
    // values add up across processes: a non-monotonic sum on the wire.
    return {
      ...head,
      sum: {
        aggregationTemporality: 2,
        isMonotonic: false,
        dataPoints: points,
      },
    }
  })
  return {
    resourceMetrics: [
      {
        resource: { attributes: RESOURCE },
        scopeMetrics: [{ scope: { name: 'neo-telemetry' }, metrics }],
      },
    ],
  }
}

function histogram(
  name: string,
  help: string,
  series: Array<[Record<string, string>, number[]]>,
): MetricFamily {
  return {
    name,
    kind: 'histogram',
    help,
    series: series.map(([labels, observations]) => ({
      labels,
      value: observations.length,
      sum: Math.round(observations.reduce((a, b) => a + b, 0) * 1000) / 1000,
      buckets: [...BOUNDS, null].map((le) => ({
        le,
        count: observations.filter((value) => le === null || value <= le)
          .length,
      })),
    })),
  }
}

const RESOURCE = [
  string('service.name', 'browseros-neo'),
  string('telemetry.sdk.name', 'opentelemetry'),
  string('telemetry.sdk.language', 'rust'),
]

function attributes(labels: Record<string, string>) {
  return Object.entries(labels).map(([key, value]) => string(key, value))
}

function string(key: string, value: string) {
  return { key, value: { stringValue: value } }
}

function int(key: string, value: number) {
  return { key, value: { intValue: String(value) } }
}

/** Epoch ms → OTLP's unix nanoseconds, a decimal string. */
function nanos(ms: number): string {
  return (BigInt(Math.round(ms * 1000)) * 1000n).toString()
}

/**
 * Families back to text exposition 0.0.4, as Neo's `GET /v1/metrics` serves them (the stand-in's
 * answer, and the parser's round-trip test). A null value is written `NaN`.
 */
export function renderPrometheus(families: MetricFamily[]): string {
  const lines: string[] = []
  const number = (value: number | null | undefined) =>
    value === null || value === undefined ? 'NaN' : String(value)
  for (const family of families) {
    if (family.help)
      lines.push(
        `# HELP ${family.name} ${family.help.replace(/\\/g, '\\\\').replace(/\n/g, '\\n')}`,
      )
    lines.push(
      `# TYPE ${family.name} ${family.kind === 'unknown' ? 'untyped' : family.kind}`,
    )
    for (const series of family.series) {
      if (family.kind !== 'histogram') {
        lines.push(
          `${family.name}${labelText(series.labels)} ${number(series.value)}`,
        )
        continue
      }
      for (const bucket of series.buckets ?? [])
        lines.push(
          `${family.name}_bucket${labelText({ ...series.labels, le: bucket.le === null ? '+Inf' : String(bucket.le) })} ${bucket.count}`,
        )
      lines.push(
        `${family.name}_sum${labelText(series.labels)} ${number(series.sum)}`,
      )
      lines.push(
        `${family.name}_count${labelText(series.labels)} ${number(series.value)}`,
      )
    }
  }
  return `${lines.join('\n')}\n`
}

function labelText(labels: Record<string, string>): string {
  const entries = Object.entries(labels)
  if (entries.length === 0) return ''
  const escaped = entries.map(
    ([key, value]) =>
      `${key}="${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n')}"`,
  )
  return `{${escaped.join(',')}}`
}
