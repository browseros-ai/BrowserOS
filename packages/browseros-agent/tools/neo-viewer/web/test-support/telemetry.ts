/**
 * Neo's telemetry as the API serves it, for the screen's tests: health, the Health view's
 * metrics, and one run's trace. Shaped after the server's fixture (`server/telemetry/fixtures.ts`:
 * a `neo.run` and eight `neo.op`, one nested, one failed), written out here so the page's tests
 * read only the API's types.
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
    families: [
      {
        name: 'neo_runs_total',
        kind: 'counter',
        help: 'Runs ended, by status and fault owner.',
        series: [
          { labels: { status: 'ok', error_owner: 'none' }, value: 14 },
          { labels: { status: 'error', error_owner: 'script' }, value: 1 },
        ],
      },
      {
        name: 'neo_run_duration_seconds',
        kind: 'histogram',
        help: 'Run duration.',
        series: [
          {
            labels: { status: 'ok' },
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

/** The fixture's trace for one run: `neo.run`, then its eight operations (seq 7 failed, 5 nested in 4). */
export function runTrace(
  neoRunId = 'neo-3',
  options: { complete?: boolean; attributes?: Record<string, string> } = {},
): RunTrace {
  const run: TraceSpan = {
    spanId: 'span-run',
    parentSpanId: 'span-viewer',
    name: 'neo.run',
    label: neoRunId,
    opId: null,
    startOffsetMs: 0,
    durationMs: 1230,
    depth: 0,
    error: false,
    attributes: { 'neo.run.id': neoRunId, 'neo.run.status': 'ok' },
  }
  const ops: Array<[number, string, number, number, number]> = [
    [1, 'page.goto', 5, 420, 1],
    [2, 'locator.textContent', 425, 450, 1],
    [3, 'locator.count', 452, 470, 1],
    [4, 'locator.click', 472, 640, 1],
    [5, 'frame.waitForLoadState', 500, 620, 2],
    [6, 'page.waitForLoadState', 642, 700, 1],
    [7, 'locator.click', 702, 1210, 1],
    [8, 'page.title', 1212, 1220, 1],
  ]
  const spans = ops.map(
    ([seq, name, start, end, depth]): TraceSpan => ({
      spanId: `span-${seq}`,
      parentSpanId: depth === 2 ? 'span-4' : 'span-run',
      name: 'neo.op',
      label: name,
      opId: `${neoRunId}.${seq}`,
      startOffsetMs: start,
      durationMs: end - start,
      depth,
      error: seq === 7,
      attributes: {
        'neo.op.name': name,
        'neo.op.id': `${neoRunId}.${seq}`,
        'neo.op.outcome': seq === 7 ? 'error' : 'ok',
        ...(seq === 7
          ? { 'neo.error.owner': 'script', 'neo.error.code': 'no_match' }
          : {}),
        ...options.attributes,
      },
    }),
  )
  const complete = options.complete ?? true
  return {
    traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
    complete,
    durationMs: 1230,
    spans: complete ? [run, ...spans] : spans,
    resource: { 'service.name': 'browseros-neo' },
  }
}
