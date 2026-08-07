import { describe, expect, it } from 'bun:test'
import {
  buildCargoJunitXml,
  parseCargoTestCounts,
  reconcileCargoCounts,
} from './run-cargo-test.helpers'

describe('parseCargoTestCounts', () => {
  it('sums passed/failed/ignored across every test binary and doctest', () => {
    const output = [
      '   Compiling browseros-core v0.1.0',
      'running 3 tests',
      'test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s',
      '',
      'running 5 tests',
      'test result: FAILED. 4 passed; 1 failed; 2 ignored; 0 measured; 0 filtered out; finished in 0.20s',
      '',
      '   Doc-tests browseros-core',
      'test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s',
    ].join('\n')

    expect(parseCargoTestCounts(output)).toEqual({
      passed: 13,
      failed: 1,
      ignored: 2,
      total: 16,
    })
  })

  it('returns zeroes when there are no result lines', () => {
    expect(parseCargoTestCounts('error: could not compile')).toEqual({
      passed: 0,
      failed: 0,
      ignored: 0,
      total: 0,
    })
  })
})

describe('reconcileCargoCounts', () => {
  const counts = (p: number, f: number, i: number) => ({
    passed: p,
    failed: f,
    ignored: i,
    total: p + f + i,
  })

  it('leaves counts untouched when cargo exited zero', () => {
    expect(reconcileCargoCounts(counts(84, 0, 0), 0)).toEqual(counts(84, 0, 0))
  })

  it('forces a failure when cargo fails before any test result (compile error)', () => {
    const result = reconcileCargoCounts(counts(0, 0, 0), 101)
    expect(result.failed).toBe(1)
    expect(result.total).toBe(1)
  })

  it('forces a failure when a later crate fails after earlier tests passed', () => {
    const result = reconcileCargoCounts(counts(84, 0, 0), 101)
    expect(result.failed).toBe(1)
    expect(result.total).toBe(84)
  })

  it('keeps the real failure count when tests actually failed', () => {
    expect(reconcileCargoCounts(counts(97, 3, 0), 101)).toEqual(
      counts(97, 3, 0),
    )
  })
})

describe('buildCargoJunitXml', () => {
  it('emits counts the summary parser reads off the testsuite element', () => {
    const xml = buildCargoJunitXml('claw-server-rust', {
      passed: 13,
      failed: 1,
      ignored: 2,
      total: 16,
    })
    expect(xml).toContain(
      '<testsuite name="claw-server-rust" tests="16" failures="1" errors="0" skipped="2">',
    )
    expect(xml).toContain('<testsuites tests="16" failures="1">')
  })
})
