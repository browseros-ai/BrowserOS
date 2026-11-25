import { describe, expect, it } from 'bun:test'
import { metricsView, quantiles } from '../test-support/telemetry'
import {
  axisEnds,
  boundText,
  bucketCounts,
  bucketsTitle,
  familyLines,
  quantileText,
  ranked,
} from './metrics'
import { lineText } from './text-lines'

describe("the Health view's numbers", () => {
  it("turns Prometheus' cumulative buckets into each bucket's own count", () => {
    const shape = quantiles([0.004, 0.2, 0.2, 0.3, 20], 0.2, 10)
    const counts = bucketCounts(shape).map((bucket) => bucket.count)
    expect(counts).toEqual([1, 0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 1])
    expect(counts.reduce((a, b) => a + b, 0)).toBe(shape.count)
  })

  it('writes bounds and quantiles as a person reads times', () => {
    expect(boundText(0.005)).toBe('5 ms')
    expect(boundText(0.25)).toBe('250 ms')
    expect(boundText(2.5)).toBe('2.5 s')
    expect(boundText(600)).toBe('10 min')
    expect(boundText(7200)).toBe('2 h')
    expect(boundText(null)).toBe('+Inf')
    expect(quantileText(quantiles([0.168], 0.168, 0.168), 0.5)).toBe('168 ms')
    expect(quantileText(quantiles([1.23], 1.23, 1.23), 0.95)).toBe('1.23 s')
    expect(quantileText(quantiles([], null, null), 0.5)).toBe('–')
  })

  it('says only "beyond the last bound" when a quantile falls in +Inf', () => {
    // Every tab lived 42 s, past Neo's last bound of 10 s: Prometheus says 10, which is a floor.
    const lifetime = quantiles(Array(8).fill(42), 10, 10)
    expect(quantileText(lifetime, 0.5)).toBe('> 10 s')
    expect(quantileText(lifetime, 0.95)).toBe('> 10 s')
    // Most fast, a few past the bound: the median is known, the tail only as beyond.
    const mixed = quantiles([...Array(19).fill(0.2), 30], 0.2, 0.25)
    expect(quantileText(mixed, 0.5)).toBe('200 ms')
    expect(quantileText(mixed, 0.95)).toBe('250 ms')
    const tail = quantiles([...Array(18).fill(0.2), 30, 30], 0.2, 10)
    expect(quantileText(tail, 0.95)).toBe('> 10 s')
    // In the bucket holding the slowest observations only its bound is known: `≤`, not a guess.
    const slow = quantiles([0.2, 0.3, 4], 0.25, 5, {
      p50Is: 'estimate',
      p95Is: 'atMost',
    })
    expect(quantileText(slow, 0.95)).toBe('≤ 5.00 s')
  })

  it("names a histogram's axis and every bucket for its tooltip", () => {
    const shape = quantiles([0.004, 20], 0.004, 10)
    expect(axisEnds(shape)).toEqual({ first: '5 ms', last: '10 s' })
    expect(bucketsTitle(shape)).toStartWith('≤ 5 ms 1 · ≤ 10 ms 0 ·')
    expect(bucketsTitle(shape)).toEndWith('≤ 10 s 0 · > 10 s 1')
  })

  it("writes the raw families in Prometheus' text form", () => {
    const lines = familyLines(metricsView().families).map(lineText)
    expect(lines).toEqual([
      '# HELP neo_runs_total Runs ended, by status and fault owner.',
      '# TYPE neo_runs_total counter',
      'neo_runs_total{neo_run_status="ok",neo_error_owner="none"} 14',
      'neo_runs_total{neo_run_status="error",neo_error_owner="script"} 1',
      '',
      '# HELP neo_run_duration_seconds Run duration.',
      '# TYPE neo_run_duration_seconds histogram',
      'neo_run_duration_seconds_bucket{neo_run_status="ok",le="1"} 0',
      'neo_run_duration_seconds_bucket{neo_run_status="ok",le="2.5"} 2',
      'neo_run_duration_seconds_bucket{neo_run_status="ok",le="+Inf"} 2',
      'neo_run_duration_seconds_sum{neo_run_status="ok"} 2.46',
      'neo_run_duration_seconds_count{neo_run_status="ok"} 2',
    ])
  })

  it('lists a by-label count largest first, then by name', () => {
    expect(ranked({ janitor: 0, script: 8, crash: 8 })).toEqual([
      ['crash', 8],
      ['script', 8],
      ['janitor', 0],
    ])
  })
})
