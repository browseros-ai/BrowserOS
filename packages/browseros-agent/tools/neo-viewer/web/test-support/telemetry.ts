/**
 * Neo's telemetry as the API serves it, for the screen's tests: health, the Health view's
 * metrics, and one run's trace. Shaped after the server's fixture (`server/telemetry/fixtures.ts`:
 * a `neo.run` and eight operation spans, one nested, one failed), written out here so the page's
 * tests read only the API's types.
 */

import type {
  MetricsView,
  Quantiles,
  RunTrace,
  TelemetryHealth,
  TraceSpan,
} from '../../shared/api'

export function health(
  overrides: Partial<TelemetryHealth> = {},
): TelemetryHealth {
  return {
    rev: 1,
    observedAt: 1_000_000,
    source: 'scrape',
    spansSeen: true,
    runsInFlight: 1,
    tabsOpen: 3,
    sinceClear: { runs: 15, runErrors: 0, ops: 120, opErrors: 16, crashes: 0 },
    ...overrides,
  }
}

const BOUNDS = [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10]

/**
 * A histogram of `observations` (seconds) at the fixture's bounds, with p50 and p95 given (and how
 * to read them: `estimate` unless said).
 */
export function quantiles(
  observations: number[],
  p50Sec: number | null,
  p95Sec: number | null,
  reading: Pick<Quantiles, 'p50Is' | 'p95Is'> = {
    p50Is: 'estimate',
    p95Is: 'estimate',
  },
): Quantiles {
  return {
    count: observations.length,
    p50Sec,
    p95Sec,
    ...reading,
    buckets: [...BOUNDS, null].map((le) => ({
      le,
      count: observations.filter((value) => le === null || value <= le).length,
    })),
  }
}

export function metricsView(): MetricsView {
  return {
    source: 'scrape',
    observedAt: 1_000_000,
    runs: {
      total: 15,
      inFlight: 1,
      byStatus: { ok: 14, error: 1 },
      duration: quantiles([...Array(14).fill(1.23), 0.61], 1.6, 2.41),
    },
    operations: [
      {
        op: 'locator.click',
        total: 21,
        errors: 11,
        duration: quantiles(
          [...Array(10).fill(0.168), ...Array(11).fill(0.508)],
          0.523,
          0.952,
        ),
      },
      {
        op: 'page.goto',
        total: 10,
        errors: 0,
        duration: quantiles(Array(10).fill(0.415), 0.375, 0.488),
      },
      {
        op: 'page.title',
        total: 10,
        errors: 0,
        duration: quantiles([], null, null),
      },
    ],
    errors: [
      { owner: 'script', code: 'no_match', count: 11 },
      { owner: 'website', code: 'navigation_failed', count: 1 },
    ],
    tabs: {
      openByOrigin: { agent: 2, found: 1 },
      openedByOrigin: { agent: 10, found: 1 },
      closedByReason: { script: 8, janitor: 0 },
      // Lifetimes past the last bound (10 s): only known to be beyond it.
      lifetime: quantiles(Array(8).fill(42), 10, 10),
    },
    crashes: 0,
    navigations: {
      total: 12,
      byStatus: { '200': 9, '404': 1, none: 2 },
      errors: { name_not_resolved: 1 },
    },
    captchas: {
      total: 3,
      solves: [
        { family: 'turnstile', verdict: 'solved', count: 2 },
        { family: 'perimeterx', verdict: 'unsupported', count: 1 },
      ],
      byReason: { checkbox_passed: 2, unsupported_type: 1 },
      spendUsd: { capsolver: 0.0012 },
    },
    families: [
      {
        name: 'neo_runs_total',
        kind: 'counter',
        help: 'Runs ended, by status and fault owner.',
        series: [
          {
            labels: { neo_run_status: 'ok', neo_error_owner: 'none' },
            value: 14,
          },
          {
            labels: { neo_run_status: 'error', neo_error_owner: 'script' },
            value: 1,
          },
        ],
      },
      {
        name: 'neo_run_duration_seconds',
        kind: 'histogram',
        help: 'Run duration.',
        series: [
          {
            labels: { neo_run_status: 'ok' },
            value: 2,
            sum: 2.46,
            buckets: [
              { le: 1, count: 0 },
              { le: 2.5, count: 2 },
              { le: null, count: 2 },
            ],
          },
        ],
      },
    ],
  }
}

/**
 * The fixture's trace for one run: `neo.run`, then its eight operations (seq 7 failed, 5 nested in
 * 4, the script's on lines 1-7). With `internal`, Neo's own work for the run as a real Neo sends it
 * (wire-contract: no source line): the `events.next` long-poll, cancelled when the run ends and
 * marked ERROR as an earlier Neo did, `page.frames` and `events.listen` per open tab, and
 * the task's page opening; the spans come in draw order, siblings by start.
 */
export function runTrace(
  neoRunId = 'neo-3',
  options: {
    complete?: boolean
    attributes?: Record<string, string>
    internal?: boolean
  } = {},
): RunTrace {
  const run: TraceSpan = {
    spanId: 'span-run',
    parentSpanId: 'span-viewer',
    type: 'run',
    name: 'neo.run',
    label: neoRunId,
    opId: null,
    startOffsetMs: 0,
    durationMs: 1230,
    depth: 0,
    error: false,
    attributes: {
      'neo.span.type': 'run',
      'neo.run.id': neoRunId,
      'neo.run.status': 'ok',
    },
  }
  type Op = [number, string, number, number, number, number | null, string]
  const script: Op[] = [
    [1, 'page.goto', 5, 420, 1, 1, 'ok'],
    [2, 'locator.textContent', 425, 450, 1, 2, 'ok'],
    [3, 'locator.count', 452, 470, 1, 3, 'ok'],
    [4, 'locator.click', 472, 640, 1, 4, 'ok'],
    [5, 'frame.waitForLoadState', 500, 620, 2, null, 'ok'],
    [6, 'page.waitForLoadState', 642, 700, 1, 5, 'ok'],
    [7, 'locator.click', 702, 1210, 1, 6, 'error'],
    [8, 'page.title', 1212, 1220, 1, 7, 'ok'],
  ]
  const internal: Op[] = options.internal
    ? [
        [9, 'context.newPage', 0, 1, 1, null, 'ok'],
        [10, 'events.next', 1, 1226, 1, null, 'cancelled'],
        [11, 'page.frames', 1, 3, 1, null, 'ok'],
        [12, 'events.listen', 2, 3, 1, null, 'ok'],
        [13, 'page.frames', 3, 4, 1, null, 'ok'],
        [14, 'events.listen', 4, 5, 1, null, 'ok'],
      ]
    : []
  const span = ([
    seq,
    name,
    start,
    end,
    depth,
    line,
    outcome,
  ]: Op): TraceSpan => ({
    spanId: `span-${seq}`,
    parentSpanId: depth === 2 ? 'span-4' : 'span-run',
    type: 'op',
    name,
    label: name,
    opId: `${neoRunId}.${seq}`,
    startOffsetMs: start,
    durationMs: end - start,
    depth,
    // As the server reads OTel status: an earlier Neo marked a cancelled op ERROR.
    error: outcome !== 'ok',
    attributes: {
      'neo.span.type': 'op',
      'neo.op.name': name,
      'neo.op.id': `${neoRunId}.${seq}`,
      ...(line === null
        ? {}
        : { 'code.line.number': line, 'code.column.number': 7 }),
      'neo.op.outcome': outcome,
      ...(outcome === 'error'
        ? { 'neo.error.owner': 'script', 'neo.error.code': 'no_match' }
        : {}),
      ...options.attributes,
    },
  })
  // Draw order: the run's children by start, each followed by what ran inside it (5 in 4).
  const children = [...internal, ...script.filter((op) => op[4] === 1)].sort(
    (a, b) => a[2] - b[2] || a[0] - b[0],
  )
  const ops = children.flatMap((op) =>
    op[0] === 4
      ? [span(op), ...script.filter((inner) => inner[4] === 2).map(span)]
      : [span(op)],
  )
  const complete = options.complete ?? true
  return {
    traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
    complete,
    durationMs: 1230,
    spans: complete ? [run, ...ops] : ops,
    resource: { 'service.name': 'browseros-neo' },
  }
}
