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
    expect(store.stats()).toMatchObject({ runs: 3, spans: 27 })
  })

  test('a flood of spans or of large attributes cannot grow it; a run waited on is kept', () => {
    const store = new SpanStore({
      maxRuns: 1_000,
      maxSpansPerRun: 100,
      maxSpans: 50,
      maxBytes: 40_000,
    })
    const release = store.pin('pinned')
    store.add(runSpans('pinned'))
    for (let i = 0; i < 200; i++) store.add(runSpans(`flood-${i}`))
    const { spans, bytes } = store.stats()
    expect(spans).toBeLessThanOrEqual(50)
    expect(bytes).toBeLessThanOrEqual(40_000)
    expect(store.get('pinned')).not.toBeNull()
    release()
    for (let i = 200; i < 210; i++) store.add(runSpans(`flood-${i}`))
    expect(store.get('pinned')).toBeNull()
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
    expect(store.stats().spans).toBe(4)
    expect(store.get('r')?.ops.size).toBe(3)
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
            labels: { status: 'ok', error_owner: 'none' },
            value: runs - runErrors,
          },
          {
            labels: { status: 'error', error_owner: 'script' },
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
  test('the scrape is shown while it is at most 10 s old, else the push, else nothing; never blended', () => {
    const store = new MetricStore()
    expect(store.active(at(0))).toBeNull()
    store.pushedFamilies(sample(5, 0), at(0))
    expect(store.active(at(1))).toBe('push')
    store.scraped(sample(5, 0), 'text', at(1))
    expect(store.active(at(2))).toBe('scrape')
    // Pushes keep coming, the scrape stops: the push takes over once the scrape is 10 s old.
    store.pushedFamilies(sample(6, 0), at(11))
    expect(store.active(at(11))).toBe('scrape')
    expect(store.active(at(11.5))).toBe('push')
    // Both quiet: nothing is live; an export still finds the last heard.
    expect(store.active(at(30))).toBeNull()
    expect(store.newest(at(30))).toBe('push')
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
          name: 'neo_runs_in_flight',
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

describe('reading the 11 metrics', () => {
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
    // 3 clicks at 0.168 s and 4 at 0.508 s: the median falls in the (0.5, 1] bucket.
    expect(view.operations[0]?.duration.p50Sec).toBeCloseTo(0.5625, 4)
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
    expect(view.families).toHaveLength(11)
  })

  test("quantiles as Prometheus' histogram_quantile: linear in the bucket, the top finite bound in +Inf", () => {
    const buckets = [
      { le: 1, count: 2 },
      { le: 2, count: 6 },
      { le: null, count: 8 },
    ]
    expect(histogramQuantile(0.5, buckets)).toBeCloseTo(1.5)
    expect(histogramQuantile(0.25, buckets)).toBeCloseTo(1)
    expect(histogramQuantile(0.95, buckets)).toBe(2)
    expect(histogramQuantile(0.5, [{ le: null, count: 0 }])).toBeNull()
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
