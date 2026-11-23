import { describe, expect, test } from 'bun:test'
import type { MetricFamily } from '../../shared/api'
import { fixtureFamilies, fixtureTraces } from './fixtures'
import { MetricStore } from './metric-store'
import {
  histogramQuantile,
  metricsView,
  quantiles,
  totalsOf,
} from './metrics-view'
import { decodeSpans, type NeoSpan } from './otlp-json'
import { SpanStore } from './span-store'

const TRACE = '4bf92f3577b34da6a3ce929d0e0e4736'

/** The fixture run's spans under another run id. */
function runSpans(
  neoRunId: string,
  ops = [0, 1, 2, 3, 4, 5, 6, 7, 8],
): NeoSpan[] {
  return decodeSpans(
    fixtureTraces(
      { neoRunId, traceId: TRACE, parentSpanId: null, startMs: 1_000 },
      { ops },
    ),
  ).items
}

describe('the span store is bounded', () => {
  test('past maxRuns the run changed longest ago goes, whole', () => {
    const store = new SpanStore({
      maxRuns: 3,
      maxSpansPerRun: 100,
      maxSpans: 1_000,
      maxBytes: 1e9,
    })
    for (const id of ['a', 'b', 'c']) store.add(runSpans(id))
    // `a` changes again: `b` is now the oldest.
    store.add(runSpans('a', [1]))
    store.add(runSpans('d'))
    expect(['a', 'b', 'c', 'd'].map((id) => store.get(id) !== null)).toEqual([
      true,
      false,
      true,
      true,
    ])
    // Whole: a kept run keeps every span it had.
    expect(store.get('c')?.ops.size).toBe(8)
    expect(store.get('c')?.run).not.toBeNull()
  })

  /** The flood runs still held, of `count`. */
  const held = (store: SpanStore, count: number) =>
    Array.from({ length: count }, (_, i) => `flood-${i}`).filter(
      (id) => store.get(id) !== null,
    )

  test('a flood of spans cannot grow it; a run waited on is kept', () => {
    const store = new SpanStore({
      maxRuns: 1_000,
      maxSpansPerRun: 100,
      maxSpans: 50,
      maxBytes: 1e9,
    })
    const release = store.pin('pinned')
    store.add(runSpans('pinned'))
    for (let i = 0; i < 200; i++) store.add(runSpans(`flood-${i}`))
    // 9 spans a run, 50 at most, the pinned run among them: the newest four floods.
    expect(held(store, 200)).toEqual([
      'flood-196',
      'flood-197',
      'flood-198',
      'flood-199',
    ])
    expect(store.get('pinned')).not.toBeNull()
    release()
    for (let i = 200; i < 210; i++) store.add(runSpans(`flood-${i}`))
    expect(store.get('pinned')).toBeNull()
  })

  test("the byte cap counts what a span holds: its attributes, its batch's resource, its error message", () => {
    const limits = (maxBytes: number) => ({
      maxRuns: 1_000,
      maxSpansPerRun: 100,
      maxSpans: 1e9,
      maxBytes,
    })
    const big = (neoRunId: string, where: 'resource' | 'message') => {
      const traces = fixtureTraces(
        { neoRunId, traceId: TRACE, parentSpanId: null, startMs: 1_000 },
        { ops: [7] },
      )
      const block = traces.resourceSpans[0]
      const span = block?.scopeSpans[0]?.spans[0]
      if (!block || !span) throw new Error('no fixture span')
      if (where === 'resource')
        // 64 attributes of 4 KiB: about 260 KB carried by each span of the batch.
        block.resource.attributes = Array.from({ length: 64 }, (_, i) => ({
          key: `big.${i}`,
          value: { stringValue: 'x'.repeat(4_096) },
        }))
      else span.status = { code: 2, message: 'm'.repeat(300_000) }
      return decodeSpans(traces).items
    }
    // A resource of about 260 KB a span against 1 MiB; a message (clipped to 4 KiB when read)
    // against 20 KB: a few runs either way, never all forty.
    for (const [where, cap] of [
      ['resource', 1024 * 1024],
      ['message', 20_000],
    ] as const) {
      const store = new SpanStore(limits(cap))
      for (let i = 0; i < 40; i++) store.add(big(`flood-${i}`, where))
      const kept = held(store, 40)
      expect(kept.length).toBeGreaterThan(0)
      expect(kept.length).toBeLessThanOrEqual(4)
      expect(kept.at(-1)).toBe('flood-39')
    }
  })

  test("a run's spans over its cap are refused, its own neo.run still fits; a retried span replaces, never adds", () => {
    const store = new SpanStore({
      maxRuns: 10,
      maxSpansPerRun: 4,
      maxSpans: 1_000,
      maxBytes: 1e9,
    })
    const first = store.add(runSpans('r', [1, 2, 3, 4, 5]))
    expect(first.rejected).toBe(2)
    expect(store.add(runSpans('r', [0])).rejected).toBe(0)
    expect(store.add(runSpans('r', [1, 2, 3, 0])).rejected).toBe(0)
    expect(store.get('r')?.ops.size).toBe(3)
    expect(store.get('r')?.run).not.toBeNull()
    expect(store.spansSeen).toBe(true)
  })

  test("a run is found by its neo.run's parent: the span id the viewer minted", () => {
    const store = new SpanStore()
    store.add(
      decodeSpans(
        fixtureTraces(
          {
            neoRunId: 'cancelled',
            traceId: TRACE,
            parentSpanId: '00f067aa0ba902b7',
            startMs: 1,
          },
          { ops: [0] },
        ),
      ).items,
    )
    expect(store.runUnder('00f067aa0ba902b7')).toBe('cancelled')
    expect(store.runUnder('1111111111111111')).toBeNull()
  })
})

const at = (s: number) => 1_000_000 + s * 1_000

/** The fixture's families with some counters set. */
function sample(runs: number, runErrors: number, crashes = 0): MetricFamily[] {
  return fixtureFamilies(0).map((family) => {
    if (family.name === 'neo_runs_total')
      return {
        ...family,
        series: [
          {
            labels: { neo_run_status: 'ok', neo_error_owner: 'none' },
            value: runs - runErrors,
          },
          {
            labels: { neo_run_status: 'error', neo_error_owner: 'script' },
            value: runErrors,
          },
        ],
      }
    if (family.name === 'neo_renderer_crashes_total')
      return { ...family, series: [{ labels: {}, value: crashes }] }
    return family
  })
}

describe('the metric store', () => {
  test('the scrape is shown while it is at most 30 s old, else the push, else nothing; never blended', () => {
    const store = new MetricStore()
    expect(store.active(at(0))).toBeNull()
    store.pushedFamilies(sample(5, 0), at(0))
    expect(store.active(at(1))).toBe('push')
    store.scraped(sample(5, 0), 'text', at(1))
    expect(store.active(at(2))).toBe('scrape')
    // Pushes keep coming, the scrape stops: the push takes over once the scrape is 30 s old.
    store.pushedFamilies(sample(6, 0), at(31))
    expect(store.active(at(31))).toBe('scrape')
    expect(store.active(at(31.5))).toBe('push')
    // Both quiet: nothing is live; an export still finds the last heard.
    expect(store.active(at(70))).toBeNull()
    expect(store.newest(at(70))).toBe('push')
  })

  test('a Neo pushing at its default 15 s interval stays live between pushes', () => {
    const store = new MetricStore()
    store.pushedFamilies(sample(1, 0), at(0))
    expect(store.active(at(14.9))).toBe('push')
    store.pushedFamilies(sample(2, 0), at(15))
    expect(store.active(at(29.9))).toBe('push')
    expect(
      store.latest('push')?.families.map((family) => family.name),
    ).toContain('neo_errors_total')
  })

  test('since the last Clear: deltas from the counters as they stood, a restart (a counter going down) counts on from zero', () => {
    const store = new MetricStore()
    store.scraped(sample(100, 10, 1), '', at(0))
    // The first sample is the baseline: nothing counted yet.
    expect(store.sinceClear('scrape')).toEqual({
      runs: 0,
      runErrors: 0,
      ops: 0,
      opErrors: 0,
      crashes: 0,
    })
    store.scraped(sample(103, 11, 1), '', at(2))
    expect(store.sinceClear('scrape')).toMatchObject({
      runs: 3,
      runErrors: 1,
      crashes: 0,
    })
    // Neo restarted: its counters start again from zero, so 2 new runs are 2, not -101.
    store.scraped(sample(2, 0, 1), '', at(4))
    expect(store.sinceClear('scrape')).toMatchObject({
      runs: 5,
      runErrors: 1,
      crashes: 0,
    })
    store.rebase()
    expect(store.sinceClear('scrape').runs).toBe(0)
    store.scraped(sample(4, 1, 1), '', at(6))
    expect(store.sinceClear('scrape')).toMatchObject({
      runs: 2,
      runErrors: 1,
      crashes: 0,
    })
  })

  test("after Neo restarts, the old process's families go once the new one has not sent them for 30 s", () => {
    const store = new MetricStore()
    const names = () =>
      (store.latest('push')?.families ?? []).map((family) => family.name)
    // The old process had recorded everything.
    store.pushedFamilies(sample(40, 3, 1), at(0))
    expect(names()).toContain('neo_errors_total')
    // The new one has only its gauges and its first runs so far.
    const fresh = sample(1, 0).filter((family) =>
      ['neo_runs_total', 'neo_run_active', 'neo_tab_count'].includes(
        family.name,
      ),
    )
    store.pushedFamilies(fresh, at(5))
    expect(names()).toContain('neo_errors_total')
    store.pushedFamilies(fresh, at(30.5))
    expect(names().sort()).toEqual([
      'neo_run_active',
      'neo_runs_total',
      'neo_tab_count',
    ])
    // Not the old process's 40 runs, crashes and errors beside the new one's.
    expect(store.totals('push')).toMatchObject({
      runs: 1,
      crashes: 0,
      ops: 0,
    })
  })

  test('what pushes merge into stays bounded: families and series over all pushes, not per push', () => {
    const store = new MetricStore()
    const family = (name: string, series: number): MetricFamily => ({
      name,
      kind: 'gauge',
      help: null,
      series: Array.from({ length: series }, (_, i) => ({
        labels: { i: String(i) },
        value: i,
      })),
    })
    for (let i = 0; i < 600; i++)
      store.pushedFamilies([family(`f${i}`, 1)], at(i / 100))
    expect(store.latest('push')?.families.length).toBe(512)
    const many = new MetricStore()
    for (let i = 0; i < 3; i++)
      many.pushedFamilies([family(`big${i}`, 15_000)], at(i))
    const series = (many.latest('push')?.families ?? []).reduce(
      (total, one) => total + one.series.length,
      0,
    )
    expect(series).toBe(20_000)
  })

  test('history: one point a second, cumulative; pushes merge by family', () => {
    const store = new MetricStore()
    store.pushedFamilies(sample(1, 0), at(0))
    store.pushedFamilies(sample(2, 0), at(0.5))
    store.pushedFamilies(sample(3, 0), at(1.2))
    expect(store.points('push').map((point) => [point.at, point.runs])).toEqual(
      [
        [at(0.5), 2],
        [at(1.2), 3],
      ],
    )
    // A push carrying one family leaves the others as the last push said.
    store.pushedFamilies(
      [
        {
          name: 'neo_run_active',
          kind: 'gauge',
          help: null,
          series: [{ labels: {}, value: 4 }],
        },
      ],
      at(2),
    )
    expect(store.totals('push')).toMatchObject({
      runs: 3,
      runsInFlight: 4,
      tabsOpen: 3,
    })
  })
})

describe('reading the 16 metrics', () => {
  test('a quantile never reads above the slowest observation: in its bucket, only that bound, "at most" (live test F5)', () => {
    // Twenty runs: ten up to 30 s, eight up to 60 s, and the slowest two at 61.0 s and 62.2 s.
    const runs = [
      { le: 30, count: 10 },
      { le: 60, count: 18 },
      { le: 120, count: 20 },
      { le: null, count: 20 },
    ]
    // Interpolated, p95 would read 90 s for a slowest run of 62.2 s.
    expect(histogramQuantile(0.95, runs)).toEqual({ sec: 120, is: 'atMost' })
    // p50 sits below the slowest bucket: a true estimate, under its bound.
    const p50 = histogramQuantile(0.5, runs)
    expect(p50).toEqual({ sec: 30, is: 'estimate' })
    // One 60 s op: every quantile is "at most" its bucket's bound, never 114 s.
    const one = [
      { le: 30, count: 0 },
      { le: 120, count: 1 },
      { le: null, count: 1 },
    ]
    expect(histogramQuantile(0.95, one)).toEqual({ sec: 120, is: 'atMost' })
    expect(histogramQuantile(0.5, one)).toEqual({ sec: 120, is: 'atMost' })
    // The same through a family, as the Health view gets it.
    const view = metricsView(
      [
        {
          name: 'neo_run_duration_seconds',
          kind: 'histogram',
          help: null,
          series: [
            {
              labels: { neo_run_status: 'ok' },
              value: 20,
              sum: 700,
              buckets: runs,
            },
          ],
        },
      ],
      'scrape',
      0,
    )
    expect(view.runs.duration).toMatchObject({
      p95Sec: 120,
      p95Is: 'atMost',
      p50Sec: 30,
      p50Is: 'estimate',
    })
  })

  test('totals for health: errors are the runs and operations Neo attributed to someone', () => {
    expect(totalsOf(fixtureFamilies(1))).toEqual({
      runs: 5,
      runErrors: 1,
      ops: 4 * 7 + 5,
      opErrors: 5,
      crashes: 0,
      runsInFlight: 1,
      tabsOpen: 3,
    })
    // An older image's text: nothing of Neo's.
    expect(totalsOf([])).toEqual({
      runs: 0,
      runErrors: 0,
      ops: 0,
      opErrors: 0,
      crashes: 0,
      runsInFlight: 0,
      tabsOpen: 0,
    })
  })

  test("the Health view's tables", () => {
    const view = metricsView(fixtureFamilies(0), 'scrape', 42)
    expect(view.runs).toMatchObject({
      total: 4,
      inFlight: 0,
      byStatus: { ok: 3, error: 1 },
    })
    expect(view.operations[0]).toMatchObject({
      op: 'locator.click',
      total: 7,
      errors: 4,
    })
    // 3 clicks at 0.168 s and 4 at 0.508 s: the median falls in the (0.5, 1] bucket, which holds
    // the slowest: only "at most 1 s" is known (interpolating would say 0.56 s).
    expect(view.operations[0]?.duration).toMatchObject({
      p50Sec: 1,
      p50Is: 'atMost',
    })
    expect(view.errors).toEqual([
      { owner: 'script', code: 'no_match', count: 4 },
      { owner: 'website', code: 'navigation_failed', count: 1 },
    ])
    expect(view.tabs).toMatchObject({
      openByOrigin: { agent: 2, found: 1 },
      openedByOrigin: { agent: 3, found: 1 },
      closedByReason: { script: 1, janitor: 0 },
    })
    expect(view.tabs.lifetime.count).toBe(1)
    // A navigation without `status_code` is filed as `none`.
    expect(view.navigations).toEqual({
      total: 5,
      byStatus: { '200': 3, '404': 1, none: 1 },
      errors: { name_not_resolved: 1 },
    })
    expect(view.captchas).toEqual({
      total: 3,
      solves: [
        { family: 'turnstile', verdict: 'solved', count: 2 },
        { family: 'perimeterx', verdict: 'unsupported', count: 1 },
      ],
      byReason: { checkbox_passed: 1, token_accepted: 1, unsupported_type: 1 },
      spendUsd: { capsolver: 0.0012 },
    })
    expect(view.families).toHaveLength(16)
  })

  test("quantiles as Prometheus' histogram_quantile below the slowest observations' bucket, the top finite bound past it", () => {
    const buckets = [
      { le: 1, count: 2 },
      { le: 2, count: 6 },
      { le: null, count: 8 },
    ]
    const p50 = histogramQuantile(0.5, buckets)
    expect(p50.is).toBe('estimate')
    expect(p50.sec).toBeCloseTo(1.5)
    expect(histogramQuantile(0.25, buckets).sec).toBeCloseTo(1)
    expect(histogramQuantile(0.95, buckets)).toEqual({ sec: 2, is: 'over' })
    expect(histogramQuantile(0.5, [{ le: null, count: 0 }]).sec).toBeNull()
    // Series with different bounds sum at every bound either has.
    const merged = quantiles([
      {
        labels: {},
        value: 2,
        buckets: [
          { le: 1, count: 1 },
          { le: null, count: 2 },
        ],
      },
      {
        labels: {},
        value: 1,
        buckets: [
          { le: 2, count: 1 },
          { le: null, count: 1 },
        ],
      },
    ])
    expect(merged.buckets).toEqual([
      { le: 1, count: 1 },
      { le: 2, count: 2 },
      { le: null, count: 3 },
    ])
    expect(merged.count).toBe(3)
  })
})
