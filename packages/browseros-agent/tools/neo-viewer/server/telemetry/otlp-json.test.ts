import { describe, expect, test } from 'bun:test'
import {
  FIXTURE_OPS,
  fixtureFamilies,
  fixtureMetrics,
  fixtureSpanId,
  fixtureTraces,
} from './fixtures'
import { decodeMetrics, decodeSpans, NotOtlp } from './otlp-json'

const RUN = {
  neoRunId: '01a1cc18-ed14-78b3-a30d-2cb4ef81d9f4',
  traceId: '4bf92f3577b34da6a3ce929d0e0e4736',
  parentSpanId: '00f067aa0ba902b7',
  startMs: 1_763_650_000_123.25,
}

describe('OTLP JSON spans', () => {
  test("the fixture's neo.run and eight operation spans decode as Neo sent them", () => {
    const { items, rejected } = decodeSpans(fixtureTraces(RUN))
    expect(rejected).toBe(0)
    expect(items).toHaveLength(9)
    const run = items.find((span) => span.kind === 'run')
    expect(run).toMatchObject({
      kind: 'run',
      neoRunId: RUN.neoRunId,
      traceId: RUN.traceId,
      parentSpanId: RUN.parentSpanId,
      status: 'ok',
      startMs: RUN.startMs,
      failed: false,
      error: null,
      resource: { 'service.name': 'browseros-neo' },
    })
    const failed = items.find((span) => span.kind === 'op' && span.seq === 7)
    expect(failed).toMatchObject({
      kind: 'op',
      spanName: 'locator.click',
      opId: `${RUN.neoRunId}.7`,
      neoRunId: RUN.neoRunId,
      name: 'locator.click',
      site: { line: 6, column: 7 },
      tabId: 7,
      outcome: 'error',
      delivery: 'not_sent',
      failed: true,
      error: {
        owner: 'script',
        code: 'no_match',
        message: 'locator.click: Timeout 500ms exceeded.',
      },
    })
    const nested = items.find((span) => span.kind === 'op' && span.seq === 5)
    expect(nested).toMatchObject({
      site: null,
      parentSpanId: fixtureSpanId(RUN.neoRunId, 4),
    })
    expect(items.filter((span) => span.kind === 'op')).toHaveLength(
      FIXTURE_OPS.length,
    )
  })

  test('nanosecond times past 2^53 keep microseconds; ids are lowercased; a root has no parent', () => {
    const span = (extra: Record<string, unknown>) =>
      decodeSpans({
        resourceSpans: [
          {
            scopeSpans: [
              {
                spans: [
                  {
                    traceId: RUN.traceId.toUpperCase(),
                    spanId: 'AAAAAAAAAAAAAAAA',
                    name: 'neo.run',
                    startTimeUnixNano: '1763650000123456789',
                    // A number, as some encoders send it (it cannot keep nanoseconds).
                    endTimeUnixNano: 1763650000124456400,
                    attributes: [
                      { key: 'neo.span.type', value: { stringValue: 'run' } },
                      { key: 'neo.run.id', value: { stringValue: 'r1' } },
                    ],
                    ...extra,
                  },
                ],
              },
            ],
          },
        ],
      }).items[0]
    const decoded = span({ parentSpanId: '' })
    expect(decoded?.startMs).toBe(1763650000123.456)
    expect(decoded?.endMs).toBeGreaterThan(1763650000124)
    expect(decoded?.traceId).toBe(RUN.traceId)
    expect(decoded?.spanId).toBe('aaaaaaaaaaaaaaaa')
    expect(decoded?.parentSpanId).toBeNull()
    expect(span({ parentSpanId: '0000000000000000' })?.parentSpanId).toBeNull()
    // Without its status attribute, a run's status follows its OTel status.
    expect(span({ status: { code: 2 } })).toMatchObject({ status: 'error' })
    // Epoch doubles cannot distinguish these endpoints, but their integer difference is 1 ns.
    expect(span({ endTimeUnixNano: '1763650000123456790' })?.durationMs).toBe(
      0.000001,
    )
    expect(span({ endTimeUnixNano: '1763650000123456788' })?.durationMs).toBe(0)
  })

  test('attribute values: ints as numbers (strings past 2^53), doubles, bools, arrays and maps as JSON; long strings cut', () => {
    const { items } = decodeSpans({
      resourceSpans: [
        {
          scopeSpans: [
            {
              spans: [
                {
                  traceId: RUN.traceId,
                  spanId: 'aaaaaaaaaaaaaaaa',
                  name: 'neo.run',
                  attributes: [
                    { key: 'neo.span.type', value: { stringValue: 'run' } },
                    { key: 'neo.run.id', value: { stringValue: 'r1' } },
                    { key: 'i', value: { intValue: '42' } },
                    { key: 'big', value: { intValue: '9007199254740993' } },
                    { key: 'd', value: { doubleValue: 0.25 } },
                    { key: 'b', value: { boolValue: true } },
                    {
                      key: 'a',
                      value: {
                        arrayValue: {
                          values: [{ stringValue: 'x' }, { intValue: 1 }],
                        },
                      },
                    },
                    {
                      key: 'm',
                      value: {
                        kvlistValue: {
                          values: [{ key: 'k', value: { boolValue: false } }],
                        },
                      },
                    },
                    { key: 'long', value: { stringValue: 'x'.repeat(5000) } },
                  ],
                },
              ],
            },
          ],
        },
      ],
    })
    const attributes = items[0]?.attributes ?? {}
    expect(attributes).toMatchObject({
      i: 42,
      big: '9007199254740993',
      d: 0.25,
      b: true,
      a: '["x",1]',
      m: '{"k":false}',
    })
    expect(String(attributes.long)).toHaveLength(4097)
  })

  test('what one span may hold is bounded: an over-long key drops its attribute, a long status message is cut', () => {
    const { items } = decodeSpans({
      resourceSpans: [
        {
          scopeSpans: [
            {
              spans: [
                {
                  traceId: RUN.traceId,
                  spanId: 'aaaaaaaaaaaaaaaa',
                  name: 'neo.run',
                  status: { code: 2, message: 'm'.repeat(10_000) },
                  attributes: [
                    { key: 'neo.span.type', value: { stringValue: 'run' } },
                    { key: 'neo.run.id', value: { stringValue: 'r1' } },
                    { key: 'k'.repeat(257), value: { stringValue: 'dropped' } },
                    { key: 'k'.repeat(256), value: { stringValue: 'kept' } },
                    { key: 'neo.error.owner', value: { stringValue: 'neo' } },
                    { key: 'neo.error.code', value: { stringValue: 'x' } },
                  ],
                },
              ],
            },
          ],
        },
      ],
    })
    const span = items[0]
    expect(Object.keys(span?.attributes ?? {})).toEqual([
      'neo.span.type',
      'neo.run.id',
      'k'.repeat(256),
      'neo.error.owner',
      'neo.error.code',
    ])
    expect(span?.kind === 'run' && span.error?.message.length).toBe(4_097)
  })

  test('spans the viewer cannot file under a run are rejected and counted', () => {
    const base = {
      traceId: RUN.traceId,
      spanId: 'aaaaaaaaaaaaaaaa',
      startTimeUnixNano: '1',
      endTimeUnixNano: '2',
    }
    const typed = (type: string) => ({
      key: 'neo.span.type',
      value: { stringValue: type },
    })
    const runId = { key: 'neo.run.id', value: { stringValue: 'r1' } }
    const { items, rejected } = decodeSpans({
      resourceSpans: [
        {
          scopeSpans: [
            {
              spans: [
                { ...base, name: 'http.request' },
                // Named like Neo's run span, but a span is classified by its type, never its name.
                { ...base, name: 'neo.run', attributes: [runId] },
                { ...base, name: 'neo.run', attributes: [typed('run')] },
                {
                  ...base,
                  name: 'page.goto',
                  attributes: [
                    typed('op'),
                    { key: 'neo.op.id', value: { stringValue: 'no-seq' } },
                  ],
                },
                {
                  ...base,
                  traceId: '0'.repeat(32),
                  name: 'neo.run',
                  attributes: [typed('run'), runId],
                },
                {
                  ...base,
                  spanId: 'xyz',
                  name: 'neo.run',
                  attributes: [typed('run'), runId],
                },
                'not a span',
              ],
            },
          ],
        },
      ],
    })
    expect(items).toEqual([])
    expect(rejected).toBe(7)
  })

  test('a body that is not an export request at all', () => {
    expect(() => decodeSpans([])).toThrow(NotOtlp)
    expect(() => decodeSpans({ resourceSpans: {} })).toThrow(NotOtlp)
    expect(decodeSpans({})).toEqual({ items: [], rejected: 0 })
  })
})

describe('OTLP JSON metrics', () => {
  test("the fixture's 16 metrics decode to the families Prometheus text gives", () => {
    const { items, rejected } = decodeMetrics(fixtureMetrics(3))
    expect(rejected).toBe(0)
    expect(items).toEqual(fixtureFamilies(3))
  })

  test('OpenTelemetry names and keys are filed under the Prometheus names a scrape derives', () => {
    const counter = (name: string, keys: string[], unit = '{x}') => ({
      name,
      unit,
      sum: {
        isMonotonic: true,
        dataPoints: [
          {
            asInt: '1',
            attributes: keys.map((key) => ({
              key,
              value: { stringValue: 'x' },
            })),
          },
        ],
      },
    })
    const { items } = decodeMetrics({
      resourceMetrics: [
        {
          scopeMetrics: [
            {
              metrics: [
                counter('neo.operations', [
                  'neo.op.name',
                  'neo.op.outcome',
                  'neo.error.owner',
                ]),
                counter('neo.errors', ['neo.error.owner', 'error.type']),
                counter('neo.runs', ['neo.run.status', 'neo.error.owner']),
                counter('neo.navigations', ['http.response.status_code']),
                counter('neo.navigation.errors', ['error.type']),
                counter('neo.captcha.solves', [
                  'neo.captcha.family',
                  'neo.captcha.verdict',
                ]),
                counter('neo.captcha.reasons', ['neo.captcha.reason']),
                counter('neo.captcha.spend', ['neo.captcha.provider'], '{USD}'),
                counter('other.requests', ['error.type']),
                counter('other.sent', ['net.peer'], 'By'),
                counter('other.retries_total', ['9x']),
              ],
            },
          ],
        },
      ],
    })
    expect(
      items.map((family) => [
        family.name,
        Object.keys(family.series[0]?.labels ?? {}),
      ]),
    ).toEqual([
      [
        'neo_operations_total',
        ['neo_op_name', 'neo_op_outcome', 'neo_error_owner'],
      ],
      ['neo_errors_total', ['neo_error_owner', 'error_type']],
      ['neo_runs_total', ['neo_run_status', 'neo_error_owner']],
      ['neo_navigations_total', ['http_response_status_code']],
      ['neo_navigation_errors_total', ['error_type']],
      [
        'neo_captcha_solves_total',
        ['neo_captcha_family', 'neo_captcha_verdict'],
      ],
      ['neo_captcha_reasons_total', ['neo_captcha_reason']],
      ['neo_captcha_spend_total', ['neo_captcha_provider']],
      ['other_requests_total', ['error_type']],
      ['other_sent_bytes_total', ['net_peer']],
      ['other_retries_total', ['key_9x']],
    ])
  })

  test('counters, UpDownCounters and gauges; asInt as text, asDouble with NaN', () => {
    const { items, rejected } = decodeMetrics({
      resourceMetrics: [
        {
          scopeMetrics: [
            {
              metrics: [
                {
                  name: 'c_total',
                  sum: {
                    isMonotonic: true,
                    aggregationTemporality: 2,
                    dataPoints: [{ asInt: '5' }],
                  },
                },
                {
                  name: 'updown',
                  sum: { isMonotonic: false, dataPoints: [{ asInt: -1 }] },
                },
                {
                  name: 'g',
                  gauge: {
                    dataPoints: [
                      {
                        asDouble: 'NaN',
                        attributes: [{ key: 'k', value: { intValue: '3' } }],
                      },
                      { asDouble: 0.5 },
                      {},
                    ],
                  },
                },
                { name: 'nothing' },
                { description: 'no name' },
              ],
            },
          ],
        },
      ],
    })
    expect(items).toEqual([
      {
        name: 'c_total',
        kind: 'counter',
        help: null,
        series: [{ labels: {}, value: 5 }],
      },
      {
        name: 'updown',
        kind: 'gauge',
        help: null,
        series: [{ labels: {}, value: -1 }],
      },
      {
        name: 'g',
        kind: 'gauge',
        help: null,
        series: [
          { labels: { k: '3' }, value: null },
          { labels: {}, value: 0.5 },
        ],
      },
    ])
    // The point with no value, the metric with no data, the one with no name.
    expect(rejected).toBe(3)
  })

  test('histogram buckets: OTLP counts per bucket become cumulative `le` buckets ending in +Inf', () => {
    const { items } = decodeMetrics({
      resourceMetrics: [
        {
          scopeMetrics: [
            {
              metrics: [
                {
                  name: 'h_seconds',
                  histogram: {
                    dataPoints: [
                      {
                        count: '6',
                        sum: 3.5,
                        explicitBounds: [0.1, 1],
                        bucketCounts: ['1', '2', '3'],
                      },
                      { count: '1', explicitBounds: [1], bucketCounts: ['1'] },
                    ],
                  },
                },
              ],
            },
          ],
        },
      ],
    })
    expect(items[0]?.series).toEqual([
      {
        labels: {},
        value: 6,
        sum: 3.5,
        buckets: [
          { le: 0.1, count: 1 },
          { le: 1, count: 3 },
          { le: null, count: 6 },
        ],
      },
    ])
  })
})
