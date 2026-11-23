import { describe, expect, test } from 'bun:test'
import { fixtureSpanId, fixtureTraces } from './fixtures'
import { buildTrace, foldSpans } from './fold-spans'
import { decodeSpans } from './otlp-json'
import { SpanStore } from './span-store'

const RUN = {
  neoRunId: 'r1',
  traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
  parentSpanId: '00f067aa0ba902b7',
  startMs: 1_000_000,
}

/** The fixture's spans for these seqs (0 is `neo.run`), filed in a fresh store. */
function filed(...batches: number[][]) {
  const store = new SpanStore()
  for (const ops of batches)
    store.add(decodeSpans(fixtureTraces(RUN, { ops })).items)
  const run = store.get(RUN.neoRunId)
  if (!run) throw new Error('nothing filed')
  return run
}

const ALL = [0, 1, 2, 3, 4, 5, 6, 7, 8]

describe('foldSpans', () => {
  test('a whole run: Neo’s record shape, from spans, with what spans cannot say left empty', () => {
    const record = foldSpans(filed(ALL))
    expect(record).toMatchObject({
      status: 'ok',
      effects: null,
      startedAt: RUN.startMs,
      endedAt: RUN.startMs + 1230,
      durationMs: 1230,
      complete: true,
      tabs: [7],
      error: null,
      traceGaps: 0,
      source: 'spans',
    })
    expect(
      record.operations.map((op) => [
        op.seq,
        op.name,
        op.site?.line ?? null,
        op.startOffsetMs,
        op.durationMs,
        op.outcome,
        op.parentId,
      ]),
    ).toEqual([
      [1, 'page.goto', 1, 5, 415, 'ok', null],
      [2, 'locator.textContent', 2, 425, 25, 'ok', null],
      [3, 'locator.count', 3, 452, 18, 'ok', null],
      [4, 'locator.click', 4, 472, 168, 'ok', null],
      // No site: it ran inside the click (its parent span is op 4's).
      [5, 'frame.waitForLoadState', null, 500, 120, 'ok', 'r1.4'],
      [6, 'page.waitForLoadState', 5, 642, 58, 'ok', null],
      [7, 'locator.click', 6, 702, 508, 'error', null],
      [8, 'page.title', 7, 1212, 8, 'ok', null],
    ])
    expect(record.operations[6]).toMatchObject({
      id: 'r1.7',
      delivery: 'not_sent',
      error: {
        owner: 'script',
        code: 'no_match',
        message: 'locator.click: Timeout 500ms exceeded.',
        retryable: false,
        operationId: 'r1.7',
      },
      args: null,
      result: null,
      kind: '',
      effect: '',
      notes: [],
      hasScreenshot: false,
    })
  })

  test('before neo.run arrives: running, incomplete, timed from the earliest operation, gaps counted', () => {
    const record = foldSpans(filed([2, 4]))
    expect(record).toMatchObject({
      status: 'running',
      complete: false,
      endedAt: null,
      durationMs: null,
      startedAt: RUN.startMs + 425,
      traceGaps: 2,
    })
    expect(record.operations.map((op) => [op.seq, op.startOffsetMs])).toEqual([
      [2, 0],
      [4, 47],
    ])
  })

  test('arrival order and retried batches change nothing: the same spans fold the same', () => {
    const once = foldSpans(filed(ALL))
    expect(foldSpans(filed([8, 7, 6], [0], [5, 4, 3, 2, 1]))).toEqual(once)
    expect(foldSpans(filed(ALL, ALL, [3], [0]))).toEqual(once)
  })

  test('a failed run carries its error and Neo’s status', () => {
    const store = new SpanStore()
    const traces = fixtureTraces(RUN, { ops: [0] })
    const span = traces.resourceSpans[0]?.scopeSpans[0]?.spans[0]
    if (!span) throw new Error('the fixture holds the run span')
    span.attributes = [
      { key: 'neo.span.type', value: { stringValue: 'run' } },
      { key: 'neo.run.id', value: { stringValue: 'r1' } },
      { key: 'neo.run.status', value: { stringValue: 'timeout' } },
      { key: 'neo.error.owner', value: { stringValue: 'script' } },
      { key: 'neo.error.code', value: { stringValue: 'run_timeout' } },
    ]
    span.status = { code: 2, message: 'the run took longer than 30 s' }
    store.add(decodeSpans(traces).items)
    const run = store.get('r1')
    expect(run && foldSpans(run)).toMatchObject({
      status: 'timeout',
      operations: [],
      error: {
        owner: 'script',
        code: 'run_timeout',
        message: 'the run took longer than 30 s',
      },
    })
  })
})

describe('buildTrace', () => {
  test('the waterfall and operation record share the exact decoded nanosecond duration', () => {
    const store = new SpanStore()
    const body = fixtureTraces(RUN, { ops: [1] })
    const op = body.resourceSpans[0]?.scopeSpans[0]?.spans[0]
    if (!op) throw new Error('Missing fixture operation')
    op.startTimeUnixNano = '1763650000000000123'
    op.endTimeUnixNano = '1763650000500100123'
    store.add(decodeSpans(body).items)
    const spans = store.get(RUN.neoRunId)
    if (!spans) throw new Error('Missing retained spans')
    expect(buildTrace(spans)?.spans[0]?.durationMs).toBe(500.1)
    expect(foldSpans(spans).operations[0]?.durationMs).toBe(500.1)
  })

  test('depth first, siblings by start: neo.run, its ops, the nested one under its parent', () => {
    const trace = buildTrace(filed(ALL))
    expect(trace).toMatchObject({
      traceId: RUN.traceId,
      complete: true,
      durationMs: 1230,
      resource: { 'service.name': 'browseros-neo' },
    })
    expect(
      trace?.spans.map((span) => [
        span.label,
        span.depth,
        span.startOffsetMs,
        span.error,
      ]),
    ).toEqual([
      ['r1', 0, 0, false],
      ['page.goto', 1, 5, false],
      ['locator.textContent', 1, 425, false],
      ['locator.count', 1, 452, false],
      ['locator.click', 1, 472, false],
      ['frame.waitForLoadState', 2, 500, false],
      ['page.waitForLoadState', 1, 642, false],
      ['locator.click', 1, 702, true],
      ['page.title', 1, 1212, false],
    ])
    expect(trace?.spans[5]).toMatchObject({
      type: 'op',
      name: 'frame.waitForLoadState',
      opId: 'r1.5',
      parentSpanId: fixtureSpanId('r1', 4),
      attributes: { 'neo.op.name': 'frame.waitForLoadState', 'neo.tab.id': 7 },
    })
    expect(trace?.spans[0]).toMatchObject({
      type: 'run',
      name: 'neo.run',
      opId: null,
    })
  })

  test("a trace's failed spans are the operations whose outcome is error: a cancelled op is not one, nor the run", () => {
    const store = new SpanStore()
    const traces = fixtureTraces(RUN)
    const spans = traces.resourceSpans[0]?.scopeSpans[0]?.spans ?? []
    // Neo's internal long poll ends cancelled in every run, with its span's status ERROR.
    const cancelled = spans.find((span) =>
      span.attributes.some(
        (attribute) =>
          'stringValue' in attribute.value &&
          attribute.value.stringValue === 'page.title',
      ),
    ) as {
      attributes: Array<{ key: string; value: Record<string, string> }>
      status: unknown
    }
    cancelled.attributes = cancelled.attributes.map((attribute) =>
      attribute.key === 'neo.op.outcome'
        ? { key: attribute.key, value: { stringValue: 'cancelled' } }
        : attribute,
    )
    cancelled.attributes.push(
      { key: 'neo.error.owner', value: { stringValue: 'host' } },
      { key: 'neo.error.code', value: { stringValue: 'cancelled' } },
    )
    cancelled.status = { code: 2, message: 'the caller went away' }
    // A run that failed: its own status, not a failed span.
    const run = spans.find((span) => span.name === 'neo.run') as {
      status: unknown
    }
    run.status = { code: 2, message: 'run failed' }
    store.add(decodeSpans(traces).items)
    const trace = store.get('r1') && buildTrace(store.get('r1') as never)
    expect(
      trace?.spans.filter((span) => span.error).map((span) => span.opId),
    ).toEqual(['r1.7'])
  })

  test('before neo.run arrives, operations still sit one level in, and the trace is not complete', () => {
    const trace = buildTrace(filed([1, 2]))
    expect(trace?.complete).toBe(false)
    expect(trace?.spans.map((span) => span.depth)).toEqual([1, 1])
  })
})
