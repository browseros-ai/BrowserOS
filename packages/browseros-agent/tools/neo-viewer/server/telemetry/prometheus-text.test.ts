import { describe, expect, test } from 'bun:test'
import type { MetricFamily } from '../../shared/api'
import {
  fixtureFamilies,
  fixturePrometheus,
  renderPrometheus,
} from './fixtures'
import { parsePrometheus } from './prometheus-text'

function family(families: MetricFamily[], name: string) {
  const found = families.find((candidate) => candidate.name === name)
  if (!found) throw new Error(`no family ${name}`)
  return found
}

describe('Prometheus text 0.0.4', () => {
  test('counters and gauges, with HELP and TYPE', () => {
    const families = parsePrometheus(
      [
        '# HELP neo_runs_total Runs ended.',
        '# TYPE neo_runs_total counter',
        'neo_runs_total{neo_run_status="ok",neo_error_owner="none"} 3',
        'neo_runs_total{neo_run_status="error",neo_error_owner="script"} 1 1700000000000',
        '# TYPE neo_tab_count gauge',
        'neo_tab_count{neo_tab_origin="agent"} 2',
        'neo_tab_count{neo_tab_origin="found"} 1e0',
        '',
      ].join('\n'),
    )
    expect(family(families, 'neo_runs_total')).toEqual({
      name: 'neo_runs_total',
      kind: 'counter',
      help: 'Runs ended.',
      series: [
        { labels: { neo_run_status: 'ok', neo_error_owner: 'none' }, value: 3 },
        {
          labels: { neo_run_status: 'error', neo_error_owner: 'script' },
          value: 1,
        },
      ],
    })
    expect(
      family(families, 'neo_tab_count').series.map((s) => s.value),
    ).toEqual([2, 1])
  })

  test('a histogram: cumulative buckets read from the data, +Inf last, sum and count', () => {
    const families = parsePrometheus(
      [
        '# TYPE neo_run_duration_seconds histogram',
        'neo_run_duration_seconds_bucket{neo_run_status="ok",le="0.5"} 1',
        'neo_run_duration_seconds_bucket{neo_run_status="ok",le="+Inf"} 3',
        'neo_run_duration_seconds_bucket{neo_run_status="ok",le="2.5"} 2',
        'neo_run_duration_seconds_sum{neo_run_status="ok"} 4.2',
        'neo_run_duration_seconds_count{neo_run_status="ok"} 3',
      ].join('\n'),
    )
    expect(family(families, 'neo_run_duration_seconds')).toEqual({
      name: 'neo_run_duration_seconds',
      kind: 'histogram',
      help: null,
      series: [
        {
          labels: { neo_run_status: 'ok' },
          value: 3,
          sum: 4.2,
          buckets: [
            { le: 0.5, count: 1 },
            { le: 2.5, count: 2 },
            { le: null, count: 3 },
          ],
        },
      ],
    })
  })

  test('label escaping: backslash, quote and newline; a comma and braces inside a value', () => {
    const families = parsePrometheus(
      'neo_x{path="C:\\\\tmp",msg="say \\"hi\\"\\nbye",odd="a,b}{c"} 1\n',
    )
    expect(family(families, 'neo_x').series[0]?.labels).toEqual({
      path: 'C:\\tmp',
      msg: 'say "hi"\nbye',
      odd: 'a,b}{c',
    })
  })

  test('NaN and infinities parse; JSON cannot carry them, so they read as null', () => {
    const families = parsePrometheus(
      [
        '# TYPE g gauge',
        'g{k="nan"} NaN',
        'g{k="inf"} +Inf',
        'g{k="minus"} -Inf',
        'g{k="ok"} -0.5',
      ].join('\n'),
    )
    expect(
      family(families, 'g').series.map((s) => [s.labels.k, s.value]),
    ).toEqual([
      ['nan', null],
      ['inf', null],
      ['minus', null],
      ['ok', -0.5],
    ])
  })

  test('a line that cannot be read is skipped, never fatal', () => {
    const families = parsePrometheus(
      ['good 1', 'bad{oops 2', 'also_bad notanumber', 'fine 3'].join('\n'),
    )
    expect(families.map((f) => f.name)).toEqual(['good', 'fine'])
    expect(family(families, 'good').kind).toBe('unknown')
  })
})

describe('OpenMetrics extras are tolerated', () => {
  test('# EOF ends the text, # UNIT is skipped, a counter declared without _total keeps its canonical name, _created is dropped', () => {
    const families = parsePrometheus(
      [
        '# TYPE neo_runs counter',
        '# UNIT neo_runs runs',
        '# HELP neo_runs Runs ended.',
        'neo_runs_total{neo_run_status="ok"} 3',
        'neo_runs_created{neo_run_status="ok"} 1700000000.5',
        '# EOF',
        'after_eof 1',
      ].join('\n'),
    )
    expect(families).toEqual([
      {
        name: 'neo_runs_total',
        kind: 'counter',
        help: 'Runs ended.',
        series: [{ labels: { neo_run_status: 'ok' }, value: 3 }],
      },
    ])
  })

  test('exemplars after # are cut, timestamps ignored, quoted UTF-8 names read', () => {
    const families = parsePrometheus(
      [
        '# TYPE neo_operation_duration_seconds histogram',
        'neo_operation_duration_seconds_bucket{neo_op_name="page.goto",le="1"} 2 # {trace_id="4bf92f35"} 0.4 1700000000.1',
        'neo_operation_duration_seconds_bucket{neo_op_name="page.goto",le="+Inf"} 2',
        'neo_operation_duration_seconds_count{neo_op_name="page.goto"} 2 1700000000000',
        '{"neo.dotted.name", k="v"} 7',
      ].join('\n'),
    )
    expect(
      family(families, 'neo_operation_duration_seconds').series[0]?.buckets,
    ).toEqual([
      { le: 1, count: 2 },
      { le: null, count: 2 },
    ])
    expect(family(families, 'neo.dotted.name').series).toEqual([
      { labels: { k: 'v' }, value: 7 },
    ])
  })
})

test("rendered text parses back to the same families (the fixture's 16 metrics)", () => {
  const families = fixtureFamilies(2)
  expect(families).toHaveLength(16)
  const parsed = parsePrometheus(fixturePrometheus(2))
  expect(parsed).toEqual(families)
  expect(renderPrometheus(parsed)).toBe(fixturePrometheus(2))
})
